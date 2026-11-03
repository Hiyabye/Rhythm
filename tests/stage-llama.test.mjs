import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { stageLlama, variantsFor } from '../scripts/stage-llama.mjs';

const archive = Buffer.from('pinned archive bytes');
const pinFor = (bytes) => ({ bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') });
const pins = {
  build: 'b1',
  assets: {
    metal: { file: 'llama-b1-bin-macos-arm64.tar.gz', ...pinFor(archive) },
    vulkan: { file: 'llama-b1-bin-win-vulkan-x64.zip', ...pinFor(archive) },
    cpu: { file: 'llama-b1-bin-win-cpu-x64.zip', ...pinFor(archive) },
  },
};
const response = (bytes) => ({ ok: true, status: 200, body: [bytes] });

function stage(options) {
  const cwd = mkdtempSync(join(tmpdir(), 'rhythm-stage-llama-'));
  const fetched = [];
  const run = options.run ?? ((command, args) => {
    // otool -L output: llama-server needs libllama, which needs libggml.
    const file = args.at(-1);
    const deps = file.endsWith('llama-server') ? ['libllama.0.dylib'] : file.endsWith('libllama.0.dylib') ? ['libggml.0.dylib'] : [];
    return { stdout: `${file}:\n${deps.map((name) => `\t@rpath/${name} (compatibility version 0.0.0)`).join('\n')}\n\t/usr/lib/libSystem.B.dylib\n` };
  });
  const extract = (file, directory) => {
    const root = options.platform === 'darwin' ? join(directory, 'llama-b1') : directory;
    mkdirSync(root, { recursive: true });
    for (const name of options.archiveFiles) writeFileSync(join(root, name), name);
  };
  const promise = stageLlama({ cwd, pins, run, extract, log: () => {}, arch: options.platform === 'darwin' ? 'arm64' : 'x64',
    fetchImpl: async (url) => { fetched.push(url); return response(options.bytes ?? archive); }, ...options });
  return { cwd, fetched, promise, cleanup: () => rmSync(cwd, { recursive: true, force: true }) };
}

test('only the supported hosts have llama.cpp builds', () => {
  assert.deepEqual(variantsFor('darwin', 'arm64'), ['metal']);
  assert.deepEqual(variantsFor('win32', 'x64'), ['vulkan', 'cpu']);
  for (const [platform, arch] of [['darwin', 'x64'], ['linux', 'x64'], ['win32', 'arm64']]) {
    assert.throws(() => variantsFor(platform, arch), /macOS Apple Silicon and Windows 11 x64/);
  }
});

test('macOS stages llama-server and only the libraries it loads, as regular files', async () => {
  const { cwd, fetched, promise, cleanup } = stage({ platform: 'darwin',
    archiveFiles: ['llama-server', 'LICENSE', 'libllama.0.dylib', 'libggml.0.dylib', 'libmtmd.0.dylib', 'llama-cli'] });
  try {
    await promise;
    assert.deepEqual(fetched, ['https://github.com/ggml-org/llama.cpp/releases/download/b1/llama-b1-bin-macos-arm64.tar.gz']);
    const staged = join(cwd, 'src-tauri/binaries/llama/metal');
    assert.deepEqual(readdirSync(staged).sort(), ['LICENSE', 'libggml.0.dylib', 'libllama.0.dylib', 'llama-server']);
    assert.ok(readFileSync(join(cwd, 'target/llama-cache', pins.assets.metal.file)).equals(archive));
  } finally {
    cleanup();
  }
});

test('a verified cached archive is reused without downloading', async () => {
  const first = stage({ platform: 'darwin', archiveFiles: ['llama-server', 'LICENSE'], run: () => ({ stdout: '' }) });
  try {
    await first.promise;
    // Stale output from an earlier pin must not survive restaging.
    writeFileSync(join(first.cwd, 'src-tauri/binaries/llama/metal/old.dylib'), 'old');
    const fetched = [];
    await stageLlama({ cwd: first.cwd, platform: 'darwin', arch: 'arm64', pins, log: () => {},
      fetchImpl: async (url) => { fetched.push(url); return response(archive); },
      run: () => ({ stdout: '' }),
      extract: (file, directory) => {
        mkdirSync(join(directory, 'llama-b1'));
        for (const name of ['llama-server', 'LICENSE']) writeFileSync(join(directory, 'llama-b1', name), name);
      } });
    assert.deepEqual(fetched, []);
    assert.ok(!existsSync(join(first.cwd, 'src-tauri/binaries/llama/metal/old.dylib')));
  } finally {
    first.cleanup();
  }
});

test('an archive that does not match its pin is rejected and nothing is staged', async () => {
  for (const bytes of [Buffer.from('tampered archive bytes'), Buffer.concat([archive, Buffer.from('x')])]) {
    const { cwd, promise, cleanup } = stage({ platform: 'darwin', bytes, archiveFiles: ['llama-server'] });
    try {
      await assert.rejects(promise, /pinned/);
      assert.deepEqual(readdirSync(join(cwd, 'target/llama-cache')), []);
      assert.ok(!existsSync(join(cwd, 'src-tauri/binaries/llama/metal')));
    } finally {
      cleanup();
    }
  }
});

test('Windows stages both builds with every DLL except other tools and the VC runtime beside each', async () => {
  const { cwd, promise, cleanup } = stage({ platform: 'win32', readdir: (directory) => readdirSync(directory),
    archiveFiles: ['llama-server.exe', 'LICENSE-LLVM-OpenMP', 'llama-server-impl.dll', 'llama-cli-impl.dll',
      'llama-bench-impl.dll', 'ggml.dll', 'ggml-cpu-haswell.dll', 'ggml-vulkan.dll', 'llama-cli.exe'] });
  mkdirSync(join(cwd, 'src-tauri/binaries/vc-runtime'), { recursive: true });
  for (const name of ['msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll']) {
    writeFileSync(join(cwd, 'src-tauri/binaries/vc-runtime', name), name);
  }
  try {
    await promise;
    for (const variant of ['vulkan', 'cpu']) {
      assert.deepEqual(readdirSync(join(cwd, 'src-tauri/binaries/llama', variant)).sort(), [
        'LICENSE-LLVM-OpenMP', 'ggml-cpu-haswell.dll', 'ggml-vulkan.dll', 'ggml.dll', 'llama-server-impl.dll',
        'llama-server.exe', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll',
      ]);
    }
  } finally {
    cleanup();
  }
});
