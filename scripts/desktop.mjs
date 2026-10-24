import { spawnSync } from 'node:child_process';
import { mkdirSync, copyFileSync } from 'node:fs';
import { join } from 'node:path';

function run(command, args) {
  const result = spawnSync(command, args, {
    stdio: 'inherit',
    shell: process.platform === 'win32',
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

const mode = process.argv[2] ?? 'dev';
const args = process.argv.slice(3);
const release = mode === 'build' && !args.includes('--debug');
const rust = spawnSync('rustc', ['-vV'], { encoding: 'utf8' });
if (rust.status !== 0) {
  throw new Error('Rust toolchain not found. Install Rust and put cargo/rustc on PATH.');
}
const target = rust.stdout.match(/host: (.+)/)[1].trim();

run('cargo', ['build', '--locked', '-p', 'rhythm-ai-worker', ...(release ? ['--release'] : [])]);
const extension = process.platform === 'win32' ? '.exe' : '';
const binary = join('target', release ? 'release' : 'debug', `rhythm-ai-worker${extension}`);
mkdirSync('src-tauri/binaries', { recursive: true });
copyFileSync(binary, join('src-tauri/binaries', `rhythm-ai-worker-${target}${extension}`));
run(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['exec', 'tauri', '--', mode, ...args]);
