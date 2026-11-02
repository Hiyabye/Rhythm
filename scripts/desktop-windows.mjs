import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, copyFileSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { win32 } from 'node:path';

// Windows-only paths, so the orchestration tests behave the same on any host.
const { join, resolve, dirname } = win32;

const realFiles = { readdir: readdirSync, read: readFileSync, write: writeFileSync, remove: rmSync };

// Windows 11 x64 build: CPU (mistral.rs) and NVIDIA Vulkan (llama.cpp) workers,
// Microsoft VC runtime staging and the per-user NSIS installer. Every external
// effect is injectable so the orchestration is testable on any host.
export function desktop({ platform = process.platform, arch = process.arch, env = process.env,
  args = process.argv.slice(2), cwd = process.cwd(), spawn = spawnSync,
  copy = copyFileSync, mkdir = mkdirSync, files = realFiles, home = homedir() } = {}) {
  if (platform !== 'win32' || arch !== 'x64') throw new Error('Rhythm Windows builds require Windows 11 x64.');
  const backend = env.RHYTHM_AI_BACKEND || 'auto';
  if (!['auto', 'cpu', 'gpu'].includes(backend)) throw new Error('RHYTHM_AI_BACKEND must be auto, cpu or gpu.');
  const flags = [env.RUSTFLAGS, env.CARGO_ENCODED_RUSTFLAGS, env.CARGO_BUILD_RUSTFLAGS,
    env.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS].filter(Boolean).join(' ');
  if (/target-cpu\s*=\s*native/.test(flags)) throw new Error('Portable Windows releases cannot use target-cpu=native.');
  const [mode = 'dev', ...extra] = args;
  if (!['dev', 'build'].includes(mode)) throw new Error('Expected dev or build.');
  if (backend === 'cpu' && mode === 'build' && !extra.includes('--no-bundle')) {
    throw new Error('CPU-only builds require --no-bundle; distributable installers include both workers.');
  }
  if (extra.some((arg) => arg === '--target' || arg.startsWith('--target='))) throw new Error('Cross compilation is not supported. Use the x64 MSVC host.');
  const run = (command, argv, capture = false, commandEnv = env) => {
    const result = spawn(command, argv, { cwd, env: commandEnv, windowsHide: true, ...(capture ? { encoding: 'utf8' } : { stdio: 'inherit' }) });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(command + ' failed (' + (result.status ?? 'signal') + ').');
    return result;
  };
  const target = run('rustc', ['-vV'], true).stdout.match(/^host: (.+)$/m)?.[1]?.trim();
  if (target !== 'x86_64-pc-windows-msvc') throw new Error('Install the x86_64-pc-windows-msvc Rust toolchain.');
  const release = mode === 'build' && !extra.includes('--debug');
  const distribute = release && !extra.some((arg) => arg === '--no-bundle' || arg.startsWith('--bundles') || arg === '-b');
  const buildEnv = release ? releaseEnv(env, cwd, home) : env;
  const targetDir = resolve(cwd, env.CARGO_TARGET_DIR || 'target');
  const gpuTargetDir = resolve(cwd, env.RHYTHM_GPU_TARGET_DIR || targetDir + '-gpu');
  if (backend !== 'cpu') {
    if (gpuTargetDir.length > 32) {
      throw new Error('Set RHYTHM_GPU_TARGET_DIR to a short path (at most 32 characters), e.g. D:\\rhythm-gpu, for Vulkan shader compilation.');
    }
    if (gpuTargetDir.toLowerCase() === targetDir.toLowerCase()) {
      throw new Error('GPU and CPU target directories must be different.');
    }
  }
  // Use the explicitly selected host (windows-env.ps1 exports its own host),
  // retaining Windows PowerShell compatibility for other build environments.
  // Stage before expensive compilations so missing tools or denied scripts fail fast.
  const powershell = env.RHYTHM_POWERSHELL || 'powershell.exe';
  try {
    run(powershell, ['-NoProfile', '-File', join(cwd, 'scripts', 'stage-vc-runtime.ps1')]);
  } catch (cause) {
    throw new Error(`Microsoft VC runtime staging failed using ${powershell}. Set RHYTHM_POWERSHELL to the intended PowerShell executable if needed. The selected host's execution policy is unchanged.`, { cause });
  }
  console.log('Building Windows x64 CPU worker. Model weights are installed separately in the app.');
  run('cargo', ['build', '--locked', '-p', 'rhythm-ai-worker', ...(release ? ['--release'] : [])], false, buildEnv);
  const binary = join(targetDir, release ? 'release' : 'debug', 'rhythm-ai-worker.exe');
  const staged = join(cwd, 'src-tauri', 'binaries', 'rhythm-ai-worker-' + target + '.exe');
  mkdir(dirname(staged), { recursive: true });
  copy(binary, staged);
  if (backend !== 'cpu') {
    // Feature sets have independent build output: a GPU build must never replace
    // the known CPU fallback at the ordinary target path.
    console.log('Building NVIDIA GPU worker (Vulkan) with an independent CPU fallback.');
    run('cargo', ['build', '--locked', '-p', 'rhythm-ai-worker', '--no-default-features',
      '--features', 'gpu-inference', ...(release ? ['--release'] : [])], false,
    { ...buildEnv, CARGO_TARGET_DIR: gpuTargetDir, CMAKE_GENERATOR: 'Ninja', LLAMA_BUILD_SHARED_LIBS: '0',
      LLAMA_STATIC_CRT: release ? '1' : '0' });
    copy(join(gpuTargetDir, release ? 'release' : 'debug', 'rhythm-ai-worker.exe'),
      join(cwd, 'src-tauri', 'binaries', 'rhythm-ai-worker-gpu-' + target + '.exe'));
  }
  // A CPU-only build must not package an old GPU executable left by another build.
  const config = backend === 'cpu'
    ? ['--config', JSON.stringify({ bundle: { externalBin: ['binaries/rhythm-ai-worker'] } })]
    : [];
  // Earlier installers (other versions, validation builds) must not sit next to the new one.
  if (distribute) files.remove(join(targetDir, 'release', 'bundle'), { recursive: true, force: true });
  // Direct JS invocation preserves argument boundaries without npm.cmd/shell quoting.
  run(process.execPath, [join(cwd, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), mode, ...extra, ...config], false, buildEnv);
  if (distribute) publishInstaller({ cwd, targetDir, files, copy, mkdir });
}

// Release binaries embed source paths in panic locations; map the builder's
// home, Cargo home and checkout to neutral prefixes (the last match wins).
function releaseEnv(env, cwd, home) {
  const remaps = [
    [home, 'C:\\home'],
    [env.CARGO_HOME ?? join(home, '.cargo'), 'C:\\cargo'],
    [resolve(cwd), 'C:\\rhythm'],
  ].map(([from, to]) => `--remap-path-prefix=${from}=${to}`);
  const existing = env.CARGO_ENCODED_RUSTFLAGS
    ?? (env.RUSTFLAGS ?? '').split(/\s+/).filter(Boolean).join('\x1f');
  return { ...env, CARGO_ENCODED_RUSTFLAGS: [existing, ...remaps].filter(Boolean).join('\x1f') };
}

// The distribution folder holds exactly the installer and its checksum.
function publishInstaller({ cwd, targetDir, files, copy, mkdir }) {
  const { version } = JSON.parse(files.read(join(cwd, 'src-tauri', 'tauri.conf.json'), 'utf8'));
  const name = `Rhythm_${version}_x64-setup.exe`;
  const bundle = join(targetDir, 'release', 'bundle', 'nsis');
  const installers = files.readdir(bundle).filter((file) => file.endsWith('.exe'));
  if (installers.length !== 1 || installers[0] !== name) {
    throw new Error(`Expected exactly ${name} in ${bundle}, found: ${installers.join(', ') || 'none'}`);
  }
  const output = join(targetDir, 'release', 'distribution');
  files.remove(output, { recursive: true, force: true });
  mkdir(output, { recursive: true });
  copy(join(bundle, name), join(output, name));
  const digest = createHash('sha256').update(files.read(join(output, name))).digest('hex');
  files.write(join(output, `${name}.sha256`), `${digest}  ${name}\n`);
  console.log(`Distribution files in ${output}:\n  ${name}\n  ${name}.sha256 (${digest})`);
}
