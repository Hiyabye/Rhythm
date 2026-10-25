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
if (command === 'rustc') console.log('host: aarch64-apple-darwin');
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
    const env = { ...process.env, PATH: `${join(root, 'bin')}:${process.env.PATH}`, TEST_LOG: join(root, 'calls'), TEST_METAL_STATUS: available ? '0' : '1' };
    delete env.RHYTHM_AI_BACKEND;
    if (override !== undefined) env.RHYTHM_AI_BACKEND = override;
    const result = spawnSync(process.execPath, ['--input-type=module', '-e', `
      Object.defineProperty(process, 'platform', { value: ${JSON.stringify(platform)} });
      Object.defineProperty(process, 'arch', { value: ${JSON.stringify(arch)} });
      process.argv = [process.execPath, 'desktop.mjs', 'build', '--debug'];
      await import(${JSON.stringify(script)});
    `], { cwd: root, env, encoding: 'utf8' });
    const calls = result.status === 0 ? readFileSync(join(root, 'calls'), 'utf8').trim().split('\n').map(JSON.parse) : [];
    if (result.status === 0) assert.equal(readFileSync(join(root, 'src-tauri/binaries/rhythm-ai-worker-aarch64-apple-darwin'), 'utf8'), 'test-worker');
    return { ...result, calls };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const options = { skip: process.platform === 'win32' ? 'POSIX command stubs; real Windows builds retain CPU default' : false };
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
test('other platforms default to CPU without a Metal probe', options, () => {
  const result = build({ platform: 'linux', arch: 'x64' });
  assert.equal(result.status, 0, result.stderr);
  assert.ok(!result.calls.some(([command]) => command === 'xcrun'));
  assert.ok(!result.calls.find(([command]) => command === 'cargo').includes('--features'));
});
test('explicit CPU and CUDA overrides bypass automatic probing', options, () => {
  for (const backend of ['cpu', 'cuda']) {
    const result = build({ override: backend });
    assert.equal(result.status, 0, result.stderr);
    assert.ok(!result.calls.some(([command]) => command === 'xcrun'));
    assert.equal(result.calls.find(([command]) => command === 'cargo').includes('--features'), backend === 'cuda');
  }
});
test('invalid backends and Metal on another OS are rejected before a build', options, () => {
  assert.match(build({ override: 'invalid' }).stderr, /must be cpu, metal, or cuda/);
  assert.match(build({ platform: 'linux', override: 'metal' }).stderr, /Metal backend requires macOS/);
});
