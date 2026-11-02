import { spawnSync } from 'node:child_process';
import { mkdirSync, copyFileSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

export function desktop({ platform = process.platform, arch = process.arch, env = process.env,
  args = process.argv.slice(2), cwd = process.cwd(), spawn = spawnSync,
  copy = copyFileSync, mkdir = mkdirSync } = {}) {
  if (platform !== 'win32' || arch !== 'x64') throw new Error('Rhythm desktop requires Windows 11 x64.');
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
  console.log('Building Windows x64 CPU worker. Model weights are installed separately in the app.');
  run('cargo', ['build', '--locked', '-p', 'rhythm-ai-worker', ...(release ? ['--release'] : [])]);
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
    { ...env, CARGO_TARGET_DIR: gpuTargetDir, CMAKE_GENERATOR: 'Ninja', LLAMA_BUILD_SHARED_LIBS: '0',
      LLAMA_STATIC_CRT: release ? '1' : '0' });
    copy(join(gpuTargetDir, release ? 'release' : 'debug', 'rhythm-ai-worker.exe'),
      join(cwd, 'src-tauri', 'binaries', 'rhythm-ai-worker-gpu-' + target + '.exe'));
  }
  run('powershell.exe', ['-NoProfile', '-File', join(cwd, 'scripts', 'stage-vc-runtime.ps1')]);
  // A CPU-only build must not package an old GPU executable left by another build.
  const config = backend === 'cpu'
    ? ['--config', JSON.stringify({ bundle: { externalBin: ['binaries/rhythm-ai-worker'] } })]
    : [];
  // Direct JS invocation preserves argument boundaries without npm.cmd/shell quoting.
  run(process.execPath, [join(cwd, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), mode, ...extra, ...config]);
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) desktop();
