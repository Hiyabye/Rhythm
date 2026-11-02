import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join, resolve } from 'node:path';
import { desktop } from '../scripts/desktop.mjs';

function build(options = {}) {
  const calls = [], copies = [], environments = [];
  desktop({ platform: 'win32', arch: 'x64', env: {}, cwd: 'D:\\test folder 한글', args: ['build'],
    spawn: (command, args, options) => { calls.push([command, ...args]); environments.push(options.env); return { status: 0, stdout: 'host: x86_64-pc-windows-msvc\n' }; },
    copy: (...args) => copies.push(args), mkdir: () => {}, ...options });
  return { calls, copies, environments };
}
test('release stages an exe and preserves arguments without a shell', () => {
  const { calls, copies, environments } = build({ args: ['build', '--config', 'settings with spaces.json'] });
  assert.deepEqual(calls[1], ['cargo', 'build', '--locked', '-p', 'rhythm-ai-worker', '--release']);
  assert.match(copies[0][0], /rhythm-ai-worker\.exe$/);
  assert.match(copies[0][1], /rhythm-ai-worker-x86_64-pc-windows-msvc\.exe$/);
  assert.deepEqual(calls[2], ['cargo', 'build', '--locked', '-p', 'rhythm-ai-worker', '--no-default-features', '--features', 'gpu-inference', '--release']);
  assert.match(copies[1][1], /rhythm-ai-worker-gpu-x86_64-pc-windows-msvc\.exe$/);
  assert.equal(environments[2].CARGO_TARGET_DIR, resolve('D:\\test folder 한글', 'target') + '-gpu');
  assert.equal(environments[2].LLAMA_BUILD_SHARED_LIBS, '0');
  assert.equal(environments[2].LLAMA_STATIC_CRT, '1');
  assert.equal(environments[2].CMAKE_GENERATOR, 'Ninja');
  assert.notEqual(copies[0][0], copies[1][0]);
  assert.equal(calls[3][0], 'powershell.exe');
  assert.match(calls[3].at(-1), /stage-vc-runtime\.ps1$/);
  assert.deepEqual(calls[4].slice(-3), ['build', '--config', 'settings with spaces.json']);
});
test('debug build respects a separate target directory', () => {
  const target = 'D:\\separate target';
  const { calls, copies } = build({ args: ['build', '--debug'], env: { CARGO_TARGET_DIR: target } });
  assert.ok(!calls[1].includes('--release'));
  assert.equal(copies[0][0], join(target, 'debug', 'rhythm-ai-worker.exe'));
  assert.equal(copies[1][0], join(target + '-gpu', 'debug', 'rhythm-ai-worker.exe'));
});

test('GPU shader build can use a short independent path and rejects unsafe target reuse', () => {
  const gpuTarget = 'D:\\rhythm-gpu';
  const { copies, environments } = build({ env: { RHYTHM_GPU_TARGET_DIR: gpuTarget } });
  assert.equal(environments[2].CARGO_TARGET_DIR, gpuTarget);
  assert.equal(copies[1][0], join(gpuTarget, 'release', 'rhythm-ai-worker.exe'));
  assert.throws(() => build({ env: { RHYTHM_GPU_TARGET_DIR: 'D:\\' + 'long'.repeat(12) } }), /short path/);
  assert.throws(() => build({ env: { CARGO_TARGET_DIR: gpuTarget, RHYTHM_GPU_TARGET_DIR: gpuTarget } }), /must be different/);
});
test('unsupported host, backend and cross compilation fail before build', () => {
  for (const platform of ['darwin', 'linux']) assert.throws(() => build({ platform }), /Windows/);
  assert.throws(() => build({ arch: 'arm64' }), /Windows/);
  assert.throws(() => build({ env: { RHYTHM_AI_BACKEND: 'cuda' } }), /auto, cpu or gpu/);
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

test('CPU-only builds exclude any stale GPU sidecar from the Tauri configuration', () => {
  const { calls, copies } = build({ env: { RHYTHM_AI_BACKEND: 'cpu' }, args: ['build', '--no-bundle'] });
  assert.equal(copies.length, 1);
  assert.equal(calls.length, 4);
  assert.deepEqual(JSON.parse(calls[3].at(-1)), { bundle: { externalBin: ['binaries/rhythm-ai-worker'] } });
  assert.throws(() => build({ env: { RHYTHM_AI_BACKEND: 'cpu' } }), /installers include both/);
});

test('GPU build failure leaves the CPU fallback staged but never invokes Tauri', () => {
  let count = 0;
  const copies = [];
  assert.throws(() => build({
    spawn: () => ({ status: ++count === 3 ? 1 : 0, stdout: 'host: x86_64-pc-windows-msvc\n' }),
    copy: (...args) => copies.push(args),
  }), /failed/);
  assert.equal(count, 3);
  assert.equal(copies.length, 1);
  assert.match(copies[0][1], /rhythm-ai-worker-x86_64-pc-windows-msvc\.exe$/);
});

test('runtime staging failure prevents Tauri packaging', () => {
  let count = 0;
  assert.throws(() => build({
    spawn: (command) => {
      count++;
      return { status: command === 'powershell.exe' ? 1 : 0, stdout: 'host: x86_64-pc-windows-msvc\n' };
    },
  }), /powershell.exe failed/);
  assert.equal(count, 4);
});
