// Stages the pinned upstream llama.cpp build for the host into
// src-tauri/binaries/llama/<variant>/, which Tauri bundles as resources.
// Archives are downloaded once into target/llama-cache/ and must match the
// size and SHA-256 recorded in scripts/llama-cpp.json before anything is used.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { closeSync, copyFileSync, existsSync, mkdirSync, openSync, readFileSync, readdirSync, renameSync, rmSync, writeSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const pinned = JSON.parse(readFileSync(new URL('./llama-cpp.json', import.meta.url), 'utf8'));
// macOS ships one build (Metal with CPU fallback); Windows ships Vulkan for
// NVIDIA GPUs and an independent CPU build.
const hostVariants = { 'darwin-arm64': ['metal'], 'win32-x64': ['vulkan', 'cpu'] };
// Windows llama.cpp DLLs import the VC runtime, which is not in the archives.
const vcRuntime = ['msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll'];

export function variantsFor(platform, arch) {
  const variants = hostVariants[`${platform}-${arch}`];
  if (!variants) throw new Error('llama.cpp staging supports macOS Apple Silicon and Windows 11 x64.');
  return variants;
}

const sha256 = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');

async function download(url, destination, { bytes, sha256: expected }, fetchImpl) {
  const response = await fetchImpl(url);
  if (!response.ok) throw new Error(`Download failed (${response.status}): ${url}`);
  const partial = `${destination}.part`;
  const hash = createHash('sha256');
  let received = 0;
  const file = openSync(partial, 'w');
  let complete = false;
  try {
    for await (const chunk of response.body) {
      received += chunk.length;
      if (received > bytes) throw new Error(`Download is larger than the pinned ${bytes} bytes: ${url}`);
      hash.update(chunk);
      writeSync(file, chunk);
    }
    if (received !== bytes || hash.digest('hex') !== expected) {
      throw new Error(`Downloaded archive does not match its pinned size and SHA-256: ${url}`);
    }
    complete = true;
  } finally {
    closeSync(file);
    if (complete) renameSync(partial, destination);
    else rmSync(partial, { force: true });
  }
}

// System tar: bsdtar on macOS and in System32 on Windows reads both formats.
// On Windows, PATH may put Git's GNU tar first, which cannot read zip files.
function systemExtract(archive, directory, platform, run) {
  const tar = platform === 'win32' ? join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe') : 'tar';
  run(tar, ['-xf', archive, '-C', directory]);
}

// The libraries llama-server loads through @rpath, followed transitively.
// Archive symlinks (libX.0.dylib -> libX.0.Y.dylib) become regular files
// under the name the loader asks for.
function macLibraries(directory, run) {
  const needed = new Set();
  const pending = ['llama-server'];
  while (pending.length) {
    const output = run('otool', ['-L', join(directory, pending.pop())], true).stdout;
    for (const [, name] of output.matchAll(/^\s+@rpath\/(\S+)/gm)) {
      if (!needed.has(name)) {
        needed.add(name);
        pending.push(name);
      }
    }
  }
  return [...needed].sort();
}

// ggml loads its CPU-variant DLLs at runtime, so keep every DLL except the
// implementation libraries of the other llama.cpp tools.
function windowsFiles(directory, readdir) {
  return readdir(directory).filter((name) => /\.dll$/i.test(name) && !/^llama-(?!server-).*-impl\.dll$/i.test(name))
    .concat('llama-server.exe', 'LICENSE-LLVM-OpenMP').sort();
}

function defaultRun(command, args, capture = false) {
  const result = spawnSync(command, args, { windowsHide: true, ...(capture ? { encoding: 'utf8' } : { stdio: 'inherit' }) });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status ?? 'signal'}).`);
  return result;
}

export async function stageLlama({ cwd = process.cwd(), platform = process.platform, arch = process.arch,
  pins = pinned, fetchImpl = fetch, run = defaultRun, extract = systemExtract, readdir = readdirSync,
  copy = copyFileSync, log = console.log } = {}) {
  const variants = variantsFor(platform, arch);
  const cache = resolve(cwd, 'target', 'llama-cache');
  const output = resolve(cwd, 'src-tauri', 'binaries', 'llama');
  mkdirSync(cache, { recursive: true });
  // Remove earlier variants and builds so the bundle holds exactly this pin.
  rmSync(output, { recursive: true, force: true });
  for (const variant of variants) {
    const pin = pins.assets[variant];
    const archive = join(cache, pin.file);
    if (existsSync(archive) && sha256(archive) !== pin.sha256) rmSync(archive);
    if (!existsSync(archive)) {
      log(`Downloading llama.cpp ${pins.build} (${variant}).`);
      await download(`https://github.com/ggml-org/llama.cpp/releases/download/${pins.build}/${pin.file}`, archive, pin, fetchImpl);
    }
    const unpacked = join(cache, `${pins.build}-${variant}`);
    rmSync(unpacked, { recursive: true, force: true });
    mkdirSync(unpacked);
    extract(archive, unpacked, platform, run);
    // The macOS archive has one top-level folder; the Windows ones are flat.
    const root = platform === 'darwin' ? join(unpacked, `llama-${pins.build}`) : unpacked;
    const names = platform === 'darwin' ? ['llama-server', 'LICENSE', ...macLibraries(root, run)] : windowsFiles(root, readdir);
    const destination = join(output, variant);
    mkdirSync(destination, { recursive: true });
    for (const name of names) copy(join(root, name), join(destination, name));
    if (platform === 'win32') {
      for (const name of vcRuntime) copy(join(cwd, 'src-tauri', 'binaries', 'vc-runtime', name), join(destination, name));
    }
    rmSync(unpacked, { recursive: true, force: true });
    log(`Staged llama.cpp ${pins.build} (${variant}) in ${destination}.`);
  }
  return { build: pins.build, variants, output };
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) await stageLlama();
