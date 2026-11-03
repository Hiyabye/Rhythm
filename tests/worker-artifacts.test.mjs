import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ensureWorkerArtifact, probeWorker } from '../scripts/worker-artifacts.mjs';

const hash = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');
function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'rhythm-worker 한글 '));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'crates/ai-worker/src'), { recursive: true });
  for (const file of ['Cargo.toml', 'Cargo.lock', 'crates/ai-worker/Cargo.toml', 'crates/ai-worker/build.rs', 'crates/ai-worker/src/main.rs']) {
    writeFileSync(join(root, file), `${file}\n`);
  }
  const built = [], probes = [];
  const binary = join(root, 'built-worker.mjs');
  const staged = join(root, 'sidecars', 'worker.mjs');
  const options = { cwd: root, env: { CARGO_HOME: join(root, 'cargo-home') }, target: 'test-host', profile: 'debug',
    cargoArgs: ['build', '--locked', '-p', 'rhythm-ai-worker'], rustIdentity: 'rustc 1.90.0\nhost: test-host',
    engine: 'mistral.rs', backend: 'cpu', binary, staged, log: () => {},
    run: (command, args, capture, env) => {
      assert.equal(command, 'cargo');
      if (capture) return { stdout: 'cargo 1.90.0' };
      built.push({ args, env });
      writeWorker(binary, { build_id: env.RHYTHM_WORKER_BUILD_ID });
    },
    // Actual child processes execute authored protocol fixtures, never model
    // downloads or Cargo. The adapter is necessary for .mjs on Windows.
    spawn: (file, args, options) => {
      probes.push(file);
      assert.equal(options.windowsHide, true);
      assert.equal(options.shell, undefined);
      return spawnSync(process.execPath, [file, ...args], { ...options, env: { ...process.env, ...options.env } });
    },
  };
  return { root, options, built, probes, run: (changes = {}) => ensureWorkerArtifact({ ...options, ...changes }) };
}

function writeWorker(file, changes = {}) {
  const status = { build_id: 'development', engine: 'mistral.rs', build_backend: 'cpu', runtime_available: true, analysis_json_v1: true, ...changes };
  writeFileSync(file, `import { createInterface } from 'node:readline';
const lines = createInterface({ input: process.stdin });
lines.on('line', (line) => {
  const request = JSON.parse(line);
  const result = request.method === 'status' ? ${JSON.stringify(status)} : { shutdown: true };
  console.log(JSON.stringify({ version: 1, id: request.id, ok: true, result }));
  if (request.method === 'shutdown') lines.close();
});
`);
}

test('build stages a verified artifact and exact subsequent builds skip compilation', (t) => {
  const { options, root, built, probes, run } = fixture(t);
  const first = run();
  assert.equal(first.reused, false);
  assert.equal(built.length, 1);
  assert.equal(built[0].env.RHYTHM_WORKER_BUILD_ID, first.fingerprint);
  const manifest = JSON.parse(readFileSync(first.manifestPath, 'utf8'));
  assert.equal(manifest.sha256, hash(options.staged));
  assert.equal(manifest.status.build_id, first.fingerprint);
  assert.deepEqual(probes, [options.binary, options.staged]);
  assert.equal(run().reused, true);
  assert.equal(built.length, 1);
  // App source edits do not invalidate independently built workers.
  mkdirSync(join(root, 'src'));
  writeFileSync(join(root, 'src/App.svelte'), 'changed UI');
  assert.equal(run().reused, true);
  assert.equal(built.length, 1);
});

test('worker source, lockfile and Cargo configuration changes rebuild instead of reusing stale output', (t) => {
  const { root, built, run } = fixture(t);
  let previous = run().fingerprint;
  for (const file of ['crates/ai-worker/src/main.rs', 'Cargo.lock', 'Cargo.toml', '.cargo/config.toml']) {
    if (file.startsWith('.cargo/')) mkdirSync(join(root, '.cargo'));
    writeFileSync(join(root, file), 'changed build input');
    const result = run();
    assert.equal(result.reused, false, file);
    assert.notEqual(result.fingerprint, previous, file);
    previous = result.fingerprint;
  }
  assert.equal(built.length, 5);
});

test('profile, features, target, Rust toolchain and build options are part of artifact identity', (t) => {
  const { options, run } = fixture(t);
  const first = run().fingerprint;
  for (const changes of [
    { profile: 'release' },
    { cargoArgs: [...options.cargoArgs, '--features', 'other-feature'] },
    { target: 'different-host' },
    { rustIdentity: 'rustc 1.91.0\nhost: test-host' },
    { env: { ...options.env, RUSTFLAGS: '-C opt-level=2' } },
    { env: { ...options.env, CARGO_PROFILE_DEV_DEBUG: '0' } },
    { env: { ...options.env, CMAKE_GENERATOR: 'Ninja' } },
    { env: { ...options.env, LLAMA_STATIC_CRT: '1' } },
  ]) {
    const result = run(changes);
    assert.equal(result.reused, false);
    assert.notEqual(result.fingerprint, first);
  }
});

test('build ID does not fingerprint itself and private environment values are not saved', (t) => {
  const { options, run, built } = fixture(t);
  const env = { ...options.env, RUSTFLAGS: '-C opt-level=1', CARGO_REGISTRIES_PRIVATE_TOKEN: 'secret-token', RHYTHM_WORKER_BUILD_ID: 'ignored' };
  const first = run({ env });
  assert.equal(run({ env: { ...env, RHYTHM_WORKER_BUILD_ID: first.fingerprint } }).reused, true);
  assert.equal(built.length, 1);
  const manifest = readFileSync(first.manifestPath, 'utf8');
  assert.ok(!manifest.includes('secret-token'));
  assert.ok(!manifest.includes('-C opt-level=1'));
});

test('missing/corrupt manifests and a corrupted executable trigger a verified rebuild', (t) => {
  const { options, built, run } = fixture(t);
  const first = run();
  for (const corrupt of [
    () => rmSync(first.manifestPath),
    () => writeFileSync(first.manifestPath, '{broken'),
    () => writeFileSync(first.manifestPath, JSON.stringify({ schema: 999 })),
    () => writeFileSync(options.staged, 'damaged executable'),
  ]) {
    corrupt();
    assert.equal(run().reused, false);
  }
  assert.equal(built.length, 5);
});

test('a matching file hash cannot replace the actual build ID/runtime handshake', (t) => {
  const { options, run, built } = fixture(t);
  const first = run();
  for (const changes of [
    { build_id: 'development' }, { build_id: 'a'.repeat(64) }, { engine: 'wrong-engine' },
    { build_backend: 'vulkan' }, { runtime_available: false }, { analysis_json_v1: false },
  ]) {
    writeWorker(options.staged, { build_id: first.fingerprint, ...changes });
    const manifest = JSON.parse(readFileSync(first.manifestPath, 'utf8'));
    manifest.sha256 = hash(options.staged);
    writeFileSync(first.manifestPath, JSON.stringify(manifest));
    assert.equal(run().reused, false, JSON.stringify(changes));
  }
  assert.equal(built.length, 7);
});

test('a new worker with a bad build ID is never staged or given a manifest', (t) => {
  const { options, run } = fixture(t);
  assert.throws(() => run({ run: (...args) => {
    const result = options.run(...args);
    if (!args[2]) writeWorker(options.binary);
    return result;
  } }), /incompatible build or runtime/);
  assert.equal(existsSync(options.staged), false);
  assert.equal(existsSync(`${options.staged}.artifact.json`), false);
});

test('a changed source during Cargo and a damaged staging copy are never certified', (t) => {
  const { options, root, run } = fixture(t);
  assert.throws(() => run({ run: (...args) => {
    const result = options.run(...args);
    if (!args[2]) writeFileSync(join(root, 'Cargo.lock'), 'changed while compiling');
    return result;
  } }), /inputs changed during compilation/);
  assert.equal(existsSync(options.staged), false);
  assert.throws(() => run({ copy: (_from, to) => writeFileSync(to, 'corrupted copy') }), /changed while staging/);
  assert.equal(existsSync(`${options.staged}.artifact.json`), false);
});

test('a staged-directory dependency failure prevents certification', (t) => {
  const { options, run } = fixture(t);
  assert.throws(() => run({ spawn: (file, ...args) => file === options.staged
    ? { status: -1073741515, stdout: '' } : options.spawn(file, ...args) }), /verification exited/);
  assert.equal(existsSync(`${options.staged}.artifact.json`), false);
});

test('failed Cargo builds preserve the prior verified staged worker and its manifest', (t) => {
  const { options, run } = fixture(t);
  const first = run();
  const previousHash = hash(options.staged);
  const previousManifest = readFileSync(first.manifestPath, 'utf8');
  assert.throws(() => run({ env: { ...options.env, RUSTFLAGS: '-C opt-level=2' }, run: (...args) => {
    if (args[2]) return options.run(...args);
    throw new Error('cargo failed');
  } }), /cargo failed/);
  assert.equal(hash(options.staged), previousHash);
  assert.equal(readFileSync(first.manifestPath, 'utf8'), previousManifest);
});

test('protocol probes reject crashes, timeouts, extra output and incompatible frames', () => {
  const fingerprint = 'b'.repeat(64);
  const options = { fingerprint, engine: 'mistral.rs', backend: 'cpu', env: {}, cwd: '.' };
  for (const result of [
    { error: new Error('ETIMEDOUT') }, { status: 1, stdout: '' },
    { status: 0, stdout: 'not JSON' },
    { status: 0, stdout: JSON.stringify({ version: 2, id: 1, ok: true, result: {} }) },
    { status: 0, stdout: '{}\n{}\n{}' },
  ]) assert.throws(() => probeWorker('fixture', { ...options, spawn: () => result }), /Worker verification/);
});

test('missing source and target redirects fail before attempting to run an arbitrary old worker', (t) => {
  const { options, root, run, built, probes } = fixture(t);
  assert.throws(() => run({ env: { ...options.env, CARGO_BUILD_TARGET: 'another-target' } }), /Cross compilation/);
  rmSync(join(root, 'Cargo.lock'));
  assert.throws(() => run(), /Missing worker build input/);
  assert.equal(built.length, 0);
  assert.equal(probes.length, 0);
});
