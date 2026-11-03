import { test } from 'node:test';
import assert from 'node:assert/strict';
import { win32 } from 'node:path';
import { desktop } from '../scripts/desktop-windows.mjs';

const { join, resolve } = win32;

// In-memory stand-in for the release bundle and distribution folder.
function memoryFiles(installers = ['Rhythm_9.8.7_x64-setup.exe']) {
  const written = new Map(), removed = [];
  return {
    written, removed,
    readdir: () => installers,
    read: (path) => path.endsWith('tauri.conf.json') ? JSON.stringify({ version: '9.8.7' }) : Buffer.from('final-installer'),
    write: (path, data) => written.set(path, data),
    remove: (path) => removed.push(path),
  };
}
function build(options = {}) {
  const calls = [], copies = [], environments = [];
  const files = options.files ?? memoryFiles();
  desktop({ platform: 'win32', arch: 'x64', env: {}, cwd: 'D:\\test folder 한글', args: ['build'], home: 'C:\\Users\\builder',
    spawn: (command, args, options) => { calls.push([command, ...args]); environments.push(options.env); return { status: 0, stdout: 'host: x86_64-pc-windows-msvc\n' }; },
    // Artifact correctness has filesystem/process tests of its own. Here the
    // stand-in keeps the build/staging branch visible to orchestration tests.
    ensureWorker: ({ run, cargoArgs, env, copy, binary, staged }) => {
      run('cargo', cargoArgs, false, env);
      copy(binary, staged);
    },
    copy: (...args) => copies.push(args), mkdir: () => {}, ...options, files });
  return { calls, copies, environments, files };
}
test('release stages an exe and preserves arguments without a shell', () => {
  const { calls, copies, environments } = build({ args: ['build', '--config', 'settings with spaces.json'] });
  assert.deepEqual(calls[1], ['powershell.exe', '-NoProfile', '-File', join('D:\\test folder 한글', 'scripts', 'stage-vc-runtime.ps1')]);
  assert.deepEqual(calls[2], ['cargo', 'build', '--locked', '-p', 'rhythm-ai-worker', '--release']);
  assert.match(copies[0][0], /rhythm-ai-worker\.exe$/);
  assert.match(copies[0][1], /rhythm-ai-worker-x86_64-pc-windows-msvc\.exe$/);
  assert.deepEqual(calls[3], ['cargo', 'build', '--locked', '-p', 'rhythm-ai-worker', '--no-default-features', '--features', 'gpu-inference', '--release']);
  assert.match(copies[1][1], /rhythm-ai-worker-gpu-x86_64-pc-windows-msvc\.exe$/);
  assert.equal(environments[3].CARGO_TARGET_DIR, resolve('D:\\test folder 한글', 'target') + '-gpu');
  assert.equal(environments[3].LLAMA_BUILD_SHARED_LIBS, '0');
  assert.equal(environments[3].LLAMA_STATIC_CRT, '1');
  assert.equal(environments[3].CMAKE_GENERATOR, 'Ninja');
  assert.notEqual(copies[0][0], copies[1][0]);
  assert.deepEqual(calls[4].slice(-3), ['build', '--config', 'settings with spaces.json']);
});
test('debug build respects a separate target directory', () => {
  const target = 'D:\\separate target';
  const { calls, copies } = build({ args: ['build', '--debug'], env: { CARGO_TARGET_DIR: target } });
  assert.ok(!calls[2].includes('--release'));
  assert.equal(copies[0][0], join(target, 'debug', 'rhythm-ai-worker.exe'));
  assert.equal(copies[1][0], join(target + '-gpu', 'debug', 'rhythm-ai-worker.exe'));
});

test('GPU shader build can use a short independent path and rejects unsafe target reuse', () => {
  const gpuTarget = 'D:\\rhythm-gpu';
  const { copies, environments } = build({ env: { RHYTHM_GPU_TARGET_DIR: gpuTarget } });
  assert.equal(environments[3].CARGO_TARGET_DIR, gpuTarget);
  assert.equal(copies[1][0], join(gpuTarget, 'release', 'rhythm-ai-worker.exe'));
  assert.throws(() => build({ env: { RHYTHM_GPU_TARGET_DIR: 'D:\\' + 'long'.repeat(12) } }), /short path/);
  assert.throws(() => build({ env: { CARGO_TARGET_DIR: gpuTarget, RHYTHM_GPU_TARGET_DIR: gpuTarget } }), /must be different/);
});
test('unsupported host, backend and cross compilation fail before build', () => {
  for (const platform of ['darwin', 'linux']) assert.throws(() => build({ platform }), /Windows/);
  assert.throws(() => build({ arch: 'arm64' }), /Windows/);
  assert.throws(() => build({ env: { RHYTHM_AI_BACKEND: 'cuda' } }), /auto, cpu or gpu/);
  assert.throws(() => build({ env: { RUSTFLAGS: '-C target-cpu=native' } }), /Portable/);
  assert.throws(() => build({ env: { CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS: '-Ctarget-cpu=native' } }), /Portable/);
  assert.throws(() => build({ args: ['build', '--target=x86_64-pc-windows-gnu'] }), /Cross/);
});
test('failed worker build never stages or invokes Tauri', () => {
  let count = 0;
  assert.throws(() => build({ spawn: (command) => {
    count++;
    return { status: command === 'cargo' ? 1 : 0, stdout: 'host: x86_64-pc-windows-msvc' };
  },
    copy: () => assert.fail('staged failed build') }), /failed/);
  assert.equal(count, 3);
});

test('CPU-only builds exclude any stale GPU sidecar from the Tauri configuration', () => {
  const { calls, copies } = build({ env: { RHYTHM_AI_BACKEND: 'cpu' }, args: ['build', '--no-bundle'] });
  assert.equal(copies.length, 1);
  assert.equal(calls.length, 4);
  assert.deepEqual(JSON.parse(calls[3].at(-1)), { bundle: { externalBin: ['binaries/rhythm-ai-worker'] } });
  assert.throws(() => build({ env: { RHYTHM_AI_BACKEND: 'cpu' } }), /installers include both/);
});

test('GPU build failure leaves the CPU fallback staged but never invokes Tauri', () => {
  let count = 0;
  const copies = [];
  assert.throws(() => build({
    spawn: () => ({ status: ++count === 4 ? 1 : 0, stdout: 'host: x86_64-pc-windows-msvc\n' }),
    copy: (...args) => copies.push(args),
  }), /failed/);
  assert.equal(count, 4);
  assert.equal(copies.length, 1);
  assert.match(copies[0][1], /rhythm-ai-worker-x86_64-pc-windows-msvc\.exe$/);
});

test('runtime staging failure prevents worker builds and Tauri packaging without retry', () => {
  let count = 0;
  assert.throws(() => build({
    spawn: (command) => {
      count++;
      return { status: command === 'powershell.exe' ? 1 : 0, stdout: 'host: x86_64-pc-windows-msvc\n' };
    },
    copy: () => assert.fail('staged after runtime failure'),
  }), (error) => {
    assert.match(error.message, /runtime staging failed using powershell.exe/);
    assert.match(error.message, /RHYTHM_POWERSHELL/);
    assert.match(error.cause.message, /powershell.exe failed/);
    return true;
  });
  assert.equal(count, 2);
});

test('runtime staging uses the selected PowerShell executable without a shell or policy overrides', () => {
  const host = 'D:\\PowerShell folder 한글\\pwsh.exe';
  const callsSeen = [];
  build({ env: { RHYTHM_POWERSHELL: host }, spawn: (command, args, options) => {
    assert.equal(options.shell, undefined);
    callsSeen.push([command, ...args]);
    return { status: 0, stdout: 'host: x86_64-pc-windows-msvc\n' };
  } });
  assert.deepEqual(callsSeen[1], [host, '-NoProfile', '-File', join('D:\\test folder 한글', 'scripts', 'stage-vc-runtime.ps1')]);
});

test('a missing selected PowerShell host preserves the cause and does not fall back', () => {
  const missing = Object.assign(new Error('spawn ENOENT'), { code: 'ENOENT' });
  const commands = [];
  assert.throws(() => build({ env: { RHYTHM_POWERSHELL: 'missing-pwsh.exe' }, spawn: (command) => {
    commands.push(command);
    return command === 'rustc' ? { status: 0, stdout: 'host: x86_64-pc-windows-msvc\n' } : { error: missing };
  } }), (error) => {
    assert.match(error.message, /runtime staging failed using missing-pwsh.exe/);
    assert.equal(error.cause, missing);
    return true;
  });
  assert.deepEqual(commands, ['rustc', 'missing-pwsh.exe']);
});

test('default release publishes exactly the installer and its checksum', () => {
  const { copies, files, environments } = build();
  const output = join(resolve('D:\\test folder 한글', 'target'), 'release', 'distribution');
  assert.ok(files.removed.includes(join(resolve('D:\\test folder 한글', 'target'), 'release', 'bundle')));
  assert.deepEqual(copies.at(-1)[1], join(output, 'Rhythm_9.8.7_x64-setup.exe'));
  const checksum = files.written.get(join(output, 'Rhythm_9.8.7_x64-setup.exe.sha256'));
  assert.match(checksum, /^[0-9a-f]{64}  Rhythm_9\.8\.7_x64-setup\.exe\n$/);
  // Builder paths are remapped in every release compilation.
  assert.match(environments[2].CARGO_ENCODED_RUSTFLAGS, /--remap-path-prefix=C:\\Users\\builder=C:\\home/);
  assert.match(environments[3].CARGO_ENCODED_RUSTFLAGS, /--remap-path-prefix=/);
});

test('an unexpected installer set is never published', () => {
  for (const installers of [[], ['Rhythm_9.8.6_x64-setup.exe'], ['Rhythm_9.8.7_x64-setup.exe', 'old.exe']]) {
    assert.throws(() => build({ files: memoryFiles(installers) }), /Expected exactly Rhythm_9\.8\.7_x64-setup\.exe/);
  }
  const { files } = build({ args: ['build', '--debug'] });
  assert.equal(files.written.size, 0);
});

test('verified worker reuse skips Cargo builds and still packages both expected sidecars', () => {
  const workers = [];
  const { calls } = build({ args: ['build', '--debug'], ensureWorker: (worker) => workers.push(worker) });
  assert.equal(calls.length, 3); // rustc, runtime staging, Tauri
  assert.ok(!calls.some(([command]) => command === 'cargo'));
  assert.deepEqual(workers.map(({ backend, engine, profile, target }) => ({ backend, engine, profile, target })), [
    { backend: 'cpu', engine: 'mistral.rs', profile: 'debug', target: 'x86_64-pc-windows-msvc' },
    { backend: 'vulkan', engine: 'llama.cpp', profile: 'debug', target: 'x86_64-pc-windows-msvc' },
  ]);
  assert.equal(workers[0].rustIdentity, 'host: x86_64-pc-windows-msvc\n');
  assert.match(workers[0].staged, /rhythm-ai-worker-x86_64/);
  assert.match(workers[1].staged, /rhythm-ai-worker-gpu-x86_64/);
});

test('worker-only command supports release, debug and CPU selection without invoking Tauri', () => {
  for (const args of [['worker'], ['worker', '--debug']]) {
    const { calls, copies, files } = build({ args });
    assert.equal(calls.length, 4);
    assert.equal(copies.length, 2);
    assert.equal(files.removed.length, 0);
    assert.equal(files.written.size, 0);
    assert.equal(calls[2].includes('--release'), !args.includes('--debug'));
  }
  assert.equal(build({ args: ['worker'], env: { RHYTHM_AI_BACKEND: 'cpu' } }).copies.length, 1);
  assert.throws(() => build({ args: ['worker', '--no-bundle'] }), /only accept --debug/);
});

test('artifact verification failure cannot reach Tauri', () => {
  const commands = [];
  assert.throws(() => build({ ensureWorker: () => { throw new Error('incompatible build'); },
    spawn: (command) => { commands.push(command); return { status: 0, stdout: 'host: x86_64-pc-windows-msvc\n' }; },
  }), /incompatible build/);
  assert.deepEqual(commands, ['rustc', 'powershell.exe']);
});
