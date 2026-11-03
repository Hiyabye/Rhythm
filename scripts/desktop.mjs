import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, copyFileSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

function run(command, args, env = process.env) {
  const result = spawnSync(command, args, { stdio: 'inherit', env });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

if (process.platform === 'win32') {
  const { desktop } = await import('./desktop-windows.mjs');
  desktop();
  process.exit(0);
}
if (process.platform !== 'darwin') {
  throw new Error('Rhythm desktop builds support macOS and Windows 11 x64.');
}

const mode = process.argv[2] ?? 'dev';
const args = process.argv.slice(3);
const release = mode === 'build' && !args.includes('--debug');
const requestedBackend = process.env.RHYTHM_AI_BACKEND;
if (requestedBackend && !['cpu', 'metal'].includes(requestedBackend)) {
  throw new Error('RHYTHM_AI_BACKEND must be cpu or metal on macOS.');
}
if (requestedBackend === 'metal' && process.arch !== 'arm64') {
  throw new Error('The Metal backend requires Apple Silicon; use cpu on Intel Macs.');
}
const availableMetal = requestedBackend !== 'cpu' && process.arch === 'arm64'
  && spawnSync('xcrun', ['metal', '--version'], { stdio: 'ignore' }).status === 0;
// An explicit metal request (release CI) must never become a CPU-only worker.
if (requestedBackend === 'metal' && !availableMetal) {
  throw new Error('RHYTHM_AI_BACKEND=metal requires the Metal Toolchain (`xcrun metal --version` failed). Install it with `xcodebuild -downloadComponent metaltoolchain`.');
}
const backend = requestedBackend ?? (availableMetal ? 'metal' : 'cpu');
const rust = spawnSync('rustc', ['-vV'], { encoding: 'utf8' });
if (rust.status !== 0) {
  throw new Error('Rust toolchain not found. Install Rust and put cargo/rustc on PATH.');
}
const target = rust.stdout.match(/host: (.+)/)?.[1]?.trim();
if (!target) {
  throw new Error('Unable to determine the Rust host target.');
}

// Release binaries embed source paths in panic locations. Map the builder's
// home, Cargo home and checkout to neutral prefixes so they do not leak.
// The last matching remap wins, so the more specific prefixes come later.
function releaseEnv() {
  const home = homedir();
  const remaps = [
    [home, '/home'],
    [process.env.CARGO_HOME ?? join(home, '.cargo'), '/cargo'],
    [resolve('.'), '/rhythm'],
  ].map(([from, to]) => `--remap-path-prefix=${from}=${to}`);
  // Cargo prefers CARGO_ENCODED_RUSTFLAGS over RUSTFLAGS; keep either.
  const existing = process.env.CARGO_ENCODED_RUSTFLAGS
    ?? (process.env.RUSTFLAGS ?? '').split(/\s+/).filter(Boolean).join('\x1f');
  const flags = [existing, ...remaps].filter(Boolean).join('\x1f');
  return { ...process.env, CARGO_ENCODED_RUSTFLAGS: flags };
}
const env = release ? releaseEnv() : process.env;
// A default release build produces the distributable DMG. Custom bundle
// selections and --no-bundle builds are left alone.
const distribute = release && !args.some((arg) => arg === '--no-bundle' || arg.startsWith('--bundles') || arg === '-b');

console.log(`Building Rhythm AI worker with ${backend} backend; no toolchains or weights are installed automatically.`);
run('cargo', ['build', '--locked', '-p', 'rhythm-ai-worker',
  ...(release ? ['--release'] : []), ...(backend === 'metal' ? ['--features', 'metal'] : [])], env);
const binary = join('target', release ? 'release' : 'debug', 'rhythm-ai-worker');
mkdirSync('src-tauri/binaries', { recursive: true });
copyFileSync(binary, join('src-tauri/binaries', `rhythm-ai-worker-${target}`));
// Earlier bundles (other versions, validation builds, updater archives) must
// not sit next to the new artifact.
if (distribute) rmSync(join('target', 'release', 'bundle'), { recursive: true, force: true });
run('npm', ['exec', 'tauri', '--', mode, ...args], env);

if (distribute) {
  const { version } = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
  const arch = target.split('-')[0];
  const name = `Rhythm_${version}_${arch}.dmg`;
  const bundle = join('target', 'release', 'bundle', 'dmg');
  const dmgs = readdirSync(bundle).filter((file) => file.endsWith('.dmg'));
  if (dmgs.length !== 1 || dmgs[0] !== name) {
    throw new Error(`Expected exactly ${name} in ${bundle}, found: ${dmgs.join(', ') || 'none'}`);
  }
  // The distribution folder holds exactly the files to publish.
  const output = join('target', 'release', 'distribution');
  rmSync(output, { recursive: true, force: true });
  mkdirSync(output, { recursive: true });
  copyFileSync(join(bundle, name), join(output, name));
  const digest = createHash('sha256').update(readFileSync(join(output, name))).digest('hex');
  writeFileSync(join(output, `${name}.sha256`), `${digest}  ${name}\n`);
  console.log(`Distribution files in ${output}:\n  ${name}\n  ${name}.sha256 (${digest})`);
}
