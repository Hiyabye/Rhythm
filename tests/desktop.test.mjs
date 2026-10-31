import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const script = new URL('../scripts/desktop.mjs', import.meta.url).href;
// These tests substitute locally authored executables, never Cargo/Tauri/toolchain installs.
function build({ platform = 'darwin', arch = 'arm64', available = true, override } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'rhythm-desktop-test-'));
  try {
    mkdirSync(join(root, 'bin'));
    mkdirSync(join(root, 'target/debug'), { recursive: true });
    writeFileSync(join(root, 'target/debug/rhythm-ai-worker'), 'test-worker');
    const mock = `#!${process.execPath}
import { appendFileSync } from 'node:fs';
import { basename } from 'node:path';
const command = basename(process.argv[1]);
appendFileSync(process.env.TEST_LOG, JSON.stringify([command, ...process.argv.slice(2)]) + '\\n');
if (command === 'rustc') console.log('host: ' + process.env.TEST_RUST_HOST);
if (command === 'xcrun') process.exit(Number(process.env.TEST_METAL_STATUS));
`;
    // .mjs source keeps the stub independent of the temporary directory's package type.
    const source = join(root, 'mock.mjs');
    writeFileSync(source, mock);
    for (const command of ['rustc', 'cargo', 'npm', 'xcrun']) {
      writeFileSync(join(root, 'bin', command), `#!/bin/sh\nexec "${process.execPath}" "${source}" "$0" "$@"\n`, { mode: 0o755 });
    }
    // The wrapper forwards command name as argv[2].
    writeFileSync(source, mock.replace('process.argv[1]', 'process.argv[2]').replace('slice(2)', 'slice(3)'));
    const env = { ...process.env, PATH: `${join(root, 'bin')}:${process.env.PATH}`, TEST_LOG: join(root, 'calls'), TEST_METAL_STATUS: available ? '0' : '1', TEST_RUST_HOST: arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin' };
    delete env.RHYTHM_AI_BACKEND;
    if (override !== undefined) env.RHYTHM_AI_BACKEND = override;
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', `
      Object.defineProperty(process, 'platform', { value: ${JSON.stringify(platform)} });
      Object.defineProperty(process, 'arch', { value: ${JSON.stringify(arch)} });
      process.argv = [process.execPath, 'desktop.mjs', 'build', '--debug'];
      await import(${JSON.stringify(script)});
    `], { cwd: root, env, encoding: 'utf8' });
    const calls = result.status === 0 ? readFileSync(join(root, 'calls'), 'utf8').trim().split('\n').map(JSON.parse) : [];
    if (result.status === 0) assert.equal(readFileSync(join(root, `src-tauri/binaries/rhythm-ai-worker-${env.TEST_RUST_HOST}`), 'utf8'), 'test-worker');
    return { ...result, calls };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const options = { skip: process.platform === 'win32' ? 'POSIX command stubs' : false };
test('Apple Silicon builds use existing Metal tooling and stage the worker', options, () => {
  const result = build();
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.calls.find(([command]) => command === 'cargo'), ['cargo', 'build', '--locked', '-p', 'rhythm-ai-worker', '--features', 'metal']);
  assert.deepEqual(result.calls.at(-1), ['npm', 'exec', 'tauri', '--', 'build', '--debug']);
});
test('missing Metal tooling falls back to a CPU build without installing tools', options, () => {
  const result = build({ available: false });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.calls.find(([command]) => command === 'cargo'), ['cargo', 'build', '--locked', '-p', 'rhythm-ai-worker']);
  assert.deepEqual(result.calls.find(([command]) => command === 'xcrun'), ['xcrun', 'metal', '--version']);
});
test('Intel macOS builds stay on the CPU backend without a Metal probe', options, () => {
  const result = build({ arch: 'x64' });
  assert.equal(result.status, 0, result.stderr);
  assert.ok(!result.calls.some(([command]) => command === 'xcrun'));
  assert.ok(!result.calls.find(([command]) => command === 'cargo').includes('--features'));
});
test('Windows and Linux build hosts are rejected before invoking toolchains', options, () => {
  for (const platform of ['linux', 'win32']) {
    const result = build({ platform });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /supported on macOS only/);
    assert.deepEqual(result.calls, []);
  }
});
test('explicit CPU override skips the Metal probe; unsupported backends fail', options, () => {
  const cpu = build({ override: 'cpu' });
  assert.equal(cpu.status, 0, cpu.stderr);
  assert.ok(!cpu.calls.some(([command]) => command === 'xcrun'));
  assert.match(build({ override: 'cuda' }).stderr, /must be cpu or metal on macOS/);
});
test('Metal override is rejected on Intel macOS', options, () => {
  assert.match(build({ arch: 'x64', override: 'metal' }).stderr, /requires Apple Silicon/);
});
