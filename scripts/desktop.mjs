import { spawnSync } from 'node:child_process';
import { mkdirSync, copyFileSync } from 'node:fs';
import { join } from 'node:path';

function run(command, args) {
  const result = spawnSync(command, args, { stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

if (process.platform !== 'darwin') {
  throw new Error('Rhythm desktop builds are supported on macOS only.');
}

const mode = process.argv[2] ?? 'dev';
const args = process.argv.slice(3);
const release = mode === 'build' && !args.includes('--debug');
const requestedBackend = process.env.RHYTHM_AI_BACKEND;
const availableMetal = !requestedBackend && process.arch === 'arm64'
  && spawnSync('xcrun', ['metal', '--version'], { stdio: 'ignore' }).status === 0;
const backend = requestedBackend ?? (availableMetal ? 'metal' : 'cpu');
if (!['cpu', 'metal'].includes(backend)) {
  throw new Error('RHYTHM_AI_BACKEND must be cpu or metal on macOS.');
}
if (backend === 'metal' && process.arch !== 'arm64') {
  throw new Error('The Metal backend requires Apple Silicon; use cpu on Intel Macs.');
}
const rust = spawnSync('rustc', ['-vV'], { encoding: 'utf8' });
if (rust.status !== 0) {
  throw new Error('Rust toolchain not found. Install Rust and put cargo/rustc on PATH.');
}
const target = rust.stdout.match(/host: (.+)/)?.[1]?.trim();
if (!target) {
  throw new Error('Unable to determine the Rust host target.');
}

console.log(`Building Rhythm AI worker with ${backend} backend; no toolchains or weights are installed automatically.`);
run('cargo', ['build', '--locked', '-p', 'rhythm-ai-worker',
  ...(release ? ['--release'] : []), ...(backend === 'metal' ? ['--features', 'metal'] : [])]);
const binary = join('target', release ? 'release' : 'debug', 'rhythm-ai-worker');
mkdirSync('src-tauri/binaries', { recursive: true });
copyFileSync(binary, join('src-tauri/binaries', `rhythm-ai-worker-${target}`));
run('npm', ['exec', 'tauri', '--', mode, ...args]);
