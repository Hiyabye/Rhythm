import { spawnSync } from 'node:child_process';
import { mkdirSync, copyFileSync } from 'node:fs';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

export function desktop({ platform = process.platform, arch = process.arch, env = process.env,
  args = process.argv.slice(2), cwd = process.cwd(), spawn = spawnSync,
  copy = copyFileSync, mkdir = mkdirSync } = {}) {
  if (platform !== 'win32' || arch !== 'x64') throw new Error('Rhythm desktop requires Windows 11 x64.');
  if (env.RHYTHM_AI_BACKEND && env.RHYTHM_AI_BACKEND !== 'cpu') throw new Error('This Windows release supports the cpu backend only.');
  const flags = [env.RUSTFLAGS, env.CARGO_ENCODED_RUSTFLAGS, env.CARGO_BUILD_RUSTFLAGS,
    env.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS].filter(Boolean).join(' ');
  if (/target-cpu\s*=\s*native/.test(flags)) throw new Error('Portable Windows releases cannot use target-cpu=native.');
  const [mode = 'dev', ...extra] = args;
  if (!['dev', 'build'].includes(mode)) throw new Error('Expected dev or build.');
  if (extra.some((arg) => arg === '--target' || arg.startsWith('--target='))) throw new Error('Cross compilation is not supported. Use the x64 MSVC host.');
  const run = (command, argv, capture = false) => {
    const result = spawn(command, argv, { cwd, env, windowsHide: true, ...(capture ? { encoding: 'utf8' } : { stdio: 'inherit' }) });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(command + ' failed (' + (result.status ?? 'signal') + ').');
    return result;
  };
  const target = run('rustc', ['-vV'], true).stdout.match(/^host: (.+)$/m)?.[1]?.trim();
  if (target !== 'x86_64-pc-windows-msvc') throw new Error('Install the x86_64-pc-windows-msvc Rust toolchain.');
  const release = mode === 'build' && !extra.includes('--debug');
  console.log('Building Windows x64 CPU worker. Model weights are installed separately in the app.');
  run('cargo', ['build', '--locked', '-p', 'rhythm-ai-worker', ...(release ? ['--release'] : [])]);
  const targetDir = resolve(cwd, env.CARGO_TARGET_DIR || 'target');
  const binary = join(targetDir, release ? 'release' : 'debug', 'rhythm-ai-worker.exe');
  const staged = join(cwd, 'src-tauri', 'binaries', 'rhythm-ai-worker-' + target + '.exe');
  mkdir(dirname(staged), { recursive: true });
  copy(binary, staged);
  // Direct JS invocation preserves argument boundaries without npm.cmd/shell quoting.
  run(process.execPath, [join(cwd, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), mode, ...extra]);
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) desktop();
