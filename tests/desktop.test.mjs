import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, rmSync, existsSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const script = new URL('../scripts/desktop.mjs', import.meta.url).href;
// These tests substitute locally authored executables, never Cargo/Tauri/toolchain installs.
function build({ platform = 'darwin', arch = 'arm64', stageStatus = 0, argv = ['build', '--debug'], env: extraEnv = {}, setup, inspect } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'rhythm-desktop-test-'));
  try {
    mkdirSync(join(root, 'bin'));
    mkdirSync(join(root, 'src-tauri'));
    writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify({ version: '9.8.7' }));
    // Stand-in for the llama.cpp staging script: records the call, never downloads.
    mkdirSync(join(root, 'scripts'));
    writeFileSync(join(root, 'scripts/stage-llama.mjs'), `import { appendFileSync } from 'node:fs';
appendFileSync(process.env.TEST_LOG, JSON.stringify(['stage-llama']) + '\\n');
process.exit(${stageStatus});
`);
    setup?.(root);
    const mock = `#!${process.execPath}
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { basename } from 'node:path';
const command = basename(process.argv[1]);
appendFileSync(process.env.TEST_LOG, JSON.stringify([command, ...process.argv.slice(2)]) + '\\n');
if (command === 'npm') appendFileSync(process.env.TEST_LOG + '.env', JSON.stringify([command, process.env.CARGO_ENCODED_RUSTFLAGS ?? null, process.env.RHYTHM_UPDATER_PUBKEY ?? null]) + '\\n');
if (command === 'rustc') console.log('host: ' + process.env.TEST_RUST_HOST);
// Stand-in for a default Tauri release bundle.
if (command === 'npm' && process.argv.includes('build') && !process.argv.includes('--debug')) {
  mkdirSync('target/release/bundle/dmg', { recursive: true });
  writeFileSync('target/release/bundle/dmg/Rhythm_9.8.7_aarch64.dmg', 'final-dmg');
  // Like Tauri, the updater archive needs both the flag and an app target.
  const config = process.argv.includes('--config') ? JSON.parse(process.argv[process.argv.indexOf('--config') + 1]) : {};
  if (config.bundle?.createUpdaterArtifacts && config.bundle.targets?.includes('app')) {
    mkdirSync('target/release/bundle/macos', { recursive: true });
    writeFileSync('target/release/bundle/macos/Rhythm.app.tar.gz', 'updater-archive');
    writeFileSync('target/release/bundle/macos/Rhythm.app.tar.gz.sig', 'updater-signature');
  }
}
`;
    // .mjs source keeps the stub independent of the temporary directory's package type.
    const source = join(root, 'mock.mjs');
    for (const command of ['rustc', 'npm']) {
      writeFileSync(join(root, 'bin', command), `#!/bin/sh\nexec "${process.execPath}" "${source}" "$0" "$@"\n`, { mode: 0o755 });
    }
    // The wrapper forwards command name as argv[2].
    writeFileSync(source, mock.replace('process.argv[1]', 'process.argv[2]').replace('slice(2)', 'slice(3)'));
    const env = { ...process.env, TAURI_SIGNING_PRIVATE_KEY: '', RHYTHM_UPDATER_PUBKEY: '', ...extraEnv, PATH: `${join(root, 'bin')}:${process.env.PATH}`, TEST_LOG: join(root, 'calls'), TEST_RUST_HOST: arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin' };
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', `
      Object.defineProperty(process, 'platform', { value: ${JSON.stringify(platform)} });
      Object.defineProperty(process, 'arch', { value: ${JSON.stringify(arch)} });
      process.argv = [process.execPath, 'desktop.mjs', ...${JSON.stringify(argv)}];
      await import(${JSON.stringify(script)});
    `], { cwd: root, env, encoding: 'utf8' });
    const read = (file) => existsSync(file) ? readFileSync(file, 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse) : [];
    return { ...result, calls: read(join(root, 'calls')), envs: read(join(root, 'calls.env')), ...inspect?.(root) };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const options = { skip: process.platform === 'win32' ? 'POSIX command stubs' : false };
test('builds stage the pinned llama.cpp build before invoking Tauri', options, () => {
  const result = build();
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.calls.map(([command]) => command), ['rustc', 'stage-llama', 'npm']);
  assert.deepEqual(result.calls.at(-1), ['npm', 'exec', 'tauri', '--', 'build', '--debug']);
});
test('failed staging never invokes Tauri', options, () => {
  const result = build({ stageStatus: 1 });
  assert.notEqual(result.status, 0);
  assert.ok(!result.calls.some(([command]) => command === 'npm'));
});
// Windows hosts route to scripts/desktop-windows.mjs (tests/desktop-windows.test.mjs).
test('unsupported build hosts are rejected before staging or Tauri', options, () => {
  for (const platform of ['linux', 'freebsd']) {
    const result = build({ platform });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /support macOS and Windows 11 x64/);
    assert.deepEqual(result.calls, []);
  }
  const intel = build({ arch: 'x64' });
  assert.notEqual(intel.status, 0);
  assert.match(intel.stderr, /require Apple Silicon/);
  assert.ok(!intel.calls.some(([command]) => command !== 'rustc'));
});
test('release builds remap local source paths for the app build', options, () => {
  const result = build({ argv: ['build'] });
  assert.equal(result.status, 0, result.stderr);
  for (const [command, flags] of result.envs) {
    const parts = flags.split('\x1f');
    assert.ok(parts.includes(`--remap-path-prefix=${homedir()}=/home`), command);
    assert.ok(parts.some((flag) => flag.startsWith('--remap-path-prefix=') && flag.endsWith('=/cargo')), command);
    assert.ok(parts.some((flag) => flag.startsWith('--remap-path-prefix=') && flag.endsWith('=/rhythm')), command);
  }
  // Debug builds keep real paths for local debugging.
  const debug = build();
  assert.deepEqual(debug.envs.map(([, flags]) => flags), debug.envs.map(() => process.env.CARGO_ENCODED_RUSTFLAGS ?? null));
});
test('release builds publish only the fresh DMG and its checksum', options, () => {
  const result = build({
    argv: ['build'],
    setup: (root) => {
      // Stale output from earlier builds must not survive.
      mkdirSync(join(root, 'target/release/bundle/dmg'), { recursive: true });
      writeFileSync(join(root, 'target/release/bundle/dmg/Rhythm_0.9.2_aarch64.dmg'), 'old');
      mkdirSync(join(root, 'target/release/bundle/macos'), { recursive: true });
      writeFileSync(join(root, 'target/release/bundle/macos/Rhythm.app.tar.gz'), 'old');
      mkdirSync(join(root, 'target/release/distribution'), { recursive: true });
      writeFileSync(join(root, 'target/release/distribution/Rhythm_0.9.2_aarch64.dmg'), 'old');
    },
    inspect: (root) => ({
      distribution: readdirSync(join(root, 'target/release/distribution')).sort(),
      checksum: readFileSync(join(root, 'target/release/distribution/Rhythm_9.8.7_aarch64.dmg.sha256'), 'utf8'),
      staleBundle: existsSync(join(root, 'target/release/bundle/macos/Rhythm.app.tar.gz')),
    }),
  });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.distribution, ['Rhythm_9.8.7_aarch64.dmg', 'Rhythm_9.8.7_aarch64.dmg.sha256']);
  const digest = createHash('sha256').update('final-dmg').digest('hex');
  assert.equal(result.checksum, `${digest}  Rhythm_9.8.7_aarch64.dmg\n`);
  assert.equal(result.staleBundle, false);
});
test('custom bundle selections do not produce distribution files', options, () => {
  const result = build({ argv: ['build', '--no-bundle'], inspect: (root) => ({ distributed: existsSync(join(root, 'target/release/distribution')) }) });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.distributed, false);
});
test('a signing key adds exactly the updater archive and its signature', options, () => {
  const result = build({
    argv: ['build'],
    env: { TAURI_SIGNING_PRIVATE_KEY: 'disposable-private', RHYTHM_UPDATER_PUBKEY: 'disposable-public' },
    inspect: (root) => ({ distribution: readdirSync(join(root, 'target/release/distribution')).sort() }),
  });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.distribution, ['Rhythm.app.tar.gz', 'Rhythm.app.tar.gz.sig', 'Rhythm_9.8.7_aarch64.dmg', 'Rhythm_9.8.7_aarch64.dmg.sha256']);
  const tauri = result.calls.at(-1);
  assert.deepEqual(tauri.slice(0, 6), ['npm', 'exec', 'tauri', '--', 'build', '--config']);
  assert.deepEqual(JSON.parse(tauri[6]), { bundle: { targets: ['app', 'dmg'], createUpdaterArtifacts: true }, plugins: { updater: { pubkey: 'disposable-public' } } });
  // option_env! compiles the same key into the app.
  assert.equal(result.envs.at(-1)[2], 'disposable-public');
});
test('a half-configured updater key fails before staging', options, () => {
  for (const env of [{ TAURI_SIGNING_PRIVATE_KEY: 'disposable-private' }, { RHYTHM_UPDATER_PUBKEY: 'disposable-public' }]) {
    const result = build({ argv: ['build'], env });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /needs both TAURI_SIGNING_PRIVATE_KEY and RHYTHM_UPDATER_PUBKEY/);
    assert.deepEqual(result.calls.map(([command]) => command), ['rustc']);
  }
});
test('non-distribution builds never request updater artifacts', options, () => {
  const env = { TAURI_SIGNING_PRIVATE_KEY: 'disposable-private', RHYTHM_UPDATER_PUBKEY: 'disposable-public' };
  for (const argv of [['build', '--debug'], ['build', '--bundles', 'app'], ['build', '--no-bundle']]) {
    const result = build({ argv, env });
    assert.equal(result.status, 0, result.stderr);
    assert.ok(!result.calls.at(-1).includes('--config'), argv.join(' '));
  }
});
