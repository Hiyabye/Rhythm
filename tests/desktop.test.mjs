import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join, resolve } from 'node:path';
import { desktop } from '../scripts/desktop.mjs';

function build(options = {}) {
  const calls = [], copies = [];
  desktop({ platform: 'win32', arch: 'x64', env: {}, cwd: resolve('test folder 한글'), args: ['build'],
    spawn: (command, args) => { calls.push([command, ...args]); return { status: 0, stdout: 'host: x86_64-pc-windows-msvc\n' }; },
    copy: (...args) => copies.push(args), mkdir: () => {}, ...options });
  return { calls, copies };
}
test('release stages an exe and preserves arguments without a shell', () => {
  const { calls, copies } = build({ args: ['build', '--config', 'settings with spaces.json'] });
  assert.deepEqual(calls[1], ['cargo', 'build', '--locked', '-p', 'rhythm-ai-worker', '--release']);
  assert.match(copies[0][0], /rhythm-ai-worker\.exe$/);
  assert.match(copies[0][1], /rhythm-ai-worker-x86_64-pc-windows-msvc\.exe$/);
  assert.deepEqual(calls[2].slice(-3), ['build', '--config', 'settings with spaces.json']);
});
test('debug build respects a separate target directory', () => {
  const target = resolve('separate target');
  const { calls, copies } = build({ args: ['build', '--debug'], env: { CARGO_TARGET_DIR: target } });
  assert.ok(!calls[1].includes('--release'));
  assert.equal(copies[0][0], join(target, 'debug', 'rhythm-ai-worker.exe'));
});
test('unsupported host, backend and cross compilation fail before build', () => {
  for (const platform of ['darwin', 'linux']) assert.throws(() => build({ platform }), /Windows/);
  assert.throws(() => build({ arch: 'arm64' }), /Windows/);
  assert.throws(() => build({ env: { RHYTHM_AI_BACKEND: 'cuda' } }), /cpu/);
  assert.throws(() => build({ env: { RUSTFLAGS: '-C target-cpu=native' } }), /Portable/);
  assert.throws(() => build({ env: { CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS: '-Ctarget-cpu=native' } }), /Portable/);
  assert.throws(() => build({ args: ['build', '--target=x86_64-pc-windows-gnu'] }), /Cross/);
});
test('failed worker build never stages or invokes Tauri', () => {
  let count = 0;
  assert.throws(() => build({ spawn: () => ++count === 1 ? { status: 0, stdout: 'host: x86_64-pc-windows-msvc' } : { status: 1 },
    copy: () => assert.fail('staged failed build') }), /failed/);
  assert.equal(count, 2);
});
