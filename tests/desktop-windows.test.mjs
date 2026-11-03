import { test } from 'node:test';
import assert from 'node:assert/strict';
import { win32 } from 'node:path';
import { desktop } from '../scripts/desktop-windows.mjs';

const { join, resolve } = win32;

// In-memory stand-in for the release bundle and distribution folder.
function memoryFiles(installers = ['Rhythm_9.8.7_x64-setup.exe']) {
  const written = new Map(), removed = [];
  return {
    written, removed,
    readdir: () => installers,
    read: (path) => path.endsWith('tauri.conf.json') ? JSON.stringify({ version: '9.8.7' }) : Buffer.from('final-installer'),
    write: (path, data) => written.set(path, data),
    remove: (path) => removed.push(path),
  };
}
function build(options = {}) {
  const calls = [], copies = [], environments = [];
  const files = options.files ?? memoryFiles();
  desktop({ platform: 'win32', arch: 'x64', env: {}, cwd: 'D:\\test folder 한글', args: ['build'], home: 'C:\\Users\\builder',
    spawn: (command, args, options) => { calls.push([command, ...args]); environments.push(options.env); return { status: 0, stdout: 'host: x86_64-pc-windows-msvc\n' }; },
    copy: (...args) => copies.push(args), mkdir: () => {}, ...options, files });
  return { calls, copies, environments, files };
}
test('release stages runtime and llama.cpp, then packages without a shell', () => {
  const { calls } = build({ args: ['build', '--config', 'settings with spaces.json'] });
  assert.deepEqual(calls.map(([command]) => command), ['rustc', 'powershell.exe', process.execPath, process.execPath]);
  assert.deepEqual(calls[1], ['powershell.exe', '-NoProfile', '-File', join('D:\\test folder 한글', 'scripts', 'stage-vc-runtime.ps1')]);
  assert.deepEqual(calls[2], [process.execPath, join('D:\\test folder 한글', 'scripts', 'stage-llama.mjs')]);
  assert.deepEqual(calls[3].slice(-3), ['build', '--config', 'settings with spaces.json']);
  assert.ok(!calls.some(([command]) => command === 'cargo'), 'no AI worker is compiled');
});
test('unsupported host and cross compilation fail before build', () => {
  for (const platform of ['darwin', 'linux']) assert.throws(() => build({ platform }), /Windows/);
  assert.throws(() => build({ arch: 'arm64' }), /Windows/);
  assert.throws(() => build({ env: { RUSTFLAGS: '-C target-cpu=native' } }), /Portable/);
  assert.throws(() => build({ env: { CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS: '-Ctarget-cpu=native' } }), /Portable/);
  assert.throws(() => build({ args: ['build', '--target=x86_64-pc-windows-gnu'] }), /Cross/);
});
test('failed llama.cpp staging never invokes Tauri', () => {
  const commands = [];
  assert.throws(() => build({ spawn: (command, args) => {
    commands.push(args.at(-1));
    return { status: String(args.at(-1)).endsWith('stage-llama.mjs') ? 1 : 0, stdout: 'host: x86_64-pc-windows-msvc' };
  } }), /failed/);
  assert.equal(commands.length, 3);
});

test('runtime staging failure prevents llama.cpp staging and Tauri packaging without retry', () => {
  let count = 0;
  assert.throws(() => build({
    spawn: (command) => {
      count++;
      return { status: command === 'powershell.exe' ? 1 : 0, stdout: 'host: x86_64-pc-windows-msvc\n' };
    },
    copy: () => assert.fail('staged after runtime failure'),
  }), (error) => {
    assert.match(error.message, /runtime staging failed using powershell.exe/);
    assert.match(error.message, /RHYTHM_POWERSHELL/);
    assert.match(error.cause.message, /powershell.exe failed/);
    return true;
  });
  assert.equal(count, 2);
});

test('runtime staging uses the selected PowerShell executable without a shell or policy overrides', () => {
  const host = 'D:\\PowerShell folder 한글\\pwsh.exe';
  const callsSeen = [];
  build({ env: { RHYTHM_POWERSHELL: host }, spawn: (command, args, options) => {
    assert.equal(options.shell, undefined);
    callsSeen.push([command, ...args]);
    return { status: 0, stdout: 'host: x86_64-pc-windows-msvc\n' };
  } });
  assert.deepEqual(callsSeen[1], [host, '-NoProfile', '-File', join('D:\\test folder 한글', 'scripts', 'stage-vc-runtime.ps1')]);
});

test('a missing selected PowerShell host preserves the cause and does not fall back', () => {
  const missing = Object.assign(new Error('spawn ENOENT'), { code: 'ENOENT' });
  const commands = [];
  assert.throws(() => build({ env: { RHYTHM_POWERSHELL: 'missing-pwsh.exe' }, spawn: (command) => {
    commands.push(command);
    return command === 'rustc' ? { status: 0, stdout: 'host: x86_64-pc-windows-msvc\n' } : { error: missing };
  } }), (error) => {
    assert.match(error.message, /runtime staging failed using missing-pwsh.exe/);
    assert.equal(error.cause, missing);
    return true;
  });
  assert.deepEqual(commands, ['rustc', 'missing-pwsh.exe']);
});

test('default release publishes exactly the installer and its checksum', () => {
  const { copies, files, environments } = build();
  const output = join(resolve('D:\\test folder 한글', 'target'), 'release', 'distribution');
  assert.ok(files.removed.includes(join(resolve('D:\\test folder 한글', 'target'), 'release', 'bundle')));
  assert.deepEqual(copies.at(-1)[1], join(output, 'Rhythm_9.8.7_x64-setup.exe'));
  const checksum = files.written.get(join(output, 'Rhythm_9.8.7_x64-setup.exe.sha256'));
  assert.match(checksum, /^[0-9a-f]{64}  Rhythm_9\.8\.7_x64-setup\.exe\n$/);
  // Builder paths are remapped in the release app compilation.
  assert.match(environments[3].CARGO_ENCODED_RUSTFLAGS, /--remap-path-prefix=C:\\Users\\builder=C:\\home/);
});

test('an unexpected installer set is never published', () => {
  for (const installers of [[], ['Rhythm_9.8.6_x64-setup.exe'], ['Rhythm_9.8.7_x64-setup.exe', 'old.exe']]) {
    assert.throws(() => build({ files: memoryFiles(installers) }), /Expected exactly Rhythm_9\.8\.7_x64-setup\.exe/);
  }
  const { files } = build({ args: ['build', '--debug'] });
  assert.equal(files.written.size, 0);
});
