import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// Tauri generates an `allow-<command>` permission for every command declared in
// the app manifest (`src-tauri/build.rs`), and a capability grants a window only
// the permissions it lists. Playwright mocks IPC, so it cannot detect a window
// that lacks a command permission; these checks are what catch that drift.
const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const read = (path) => readFileSync(join(root, path), 'utf8');

// Rust function names from the `generate_handler!` list: the authoritative set
// of callable commands.
function registeredCommands() {
  const source = read('src-tauri/src/main.rs');
  const block = /generate_handler!\[([\s\S]*?)\]\)/.exec(source);
  assert.ok(block, 'generate_handler! list not found in src-tauri/src/main.rs');
  return block[1].split('\n').map((line) => line.trim().replace(/,$/, '')).filter(Boolean)
    .map((entry) => entry.split('::').pop());
}

// The manifest declares the commands Tauri generates permissions for. A command
// missing here has no permission, so no window can invoke it.
function manifestCommands() {
  const source = read('src-tauri/build.rs');
  const block = /let commands: &\[&str\] = &\[([\s\S]*?)\];/.exec(source);
  assert.ok(block, 'app manifest command list not found in src-tauri/build.rs');
  assert.match(source, /app_manifest\(/, 'src-tauri/build.rs must pass an app_manifest to tauri_build');
  return [...block[1].matchAll(/"([^"]*)"/g)].map((match) => match[1]);
}

function capabilities() {
  const dir = join(root, 'src-tauri/capabilities');
  return readdirSync(dir).filter((name) => name.endsWith('.json')).map((name) => {
    const file = `src-tauri/capabilities/${name}`;
    return { file, ...JSON.parse(read(file)) };
  });
}

// A capability entry is either an app command permission (`allow-foo-bar`,
// slugified from `foo_bar`) or a core/plugin permission (`dialog:allow-open`).
const pluginPermission = /^[a-z-]+:/;

function appCommands(capability) {
  return capability.permissions
    .filter((permission) => !pluginPermission.test(permission))
    .map((permission) => {
      const match = /^(?:allow|deny)-(.+)$/.exec(permission);
      assert.ok(match, `${capability.file}: ${permission} is neither a core/plugin permission nor allow-*/deny-*`);
      return match[1].replace(/-/g, '_');
    });
}

function pluginPermissions(capability) {
  return capability.permissions.filter((permission) => pluginPermission.test(permission));
}

function sourceFiles() {
  const files = [];
  const walk = (dir) => {
    for (const entry of readdirSync(join(root, dir), { withFileTypes: true })) {
      const path = `${dir}/${entry.name}`;
      if (entry.isDirectory()) walk(path);
      else if (/\.(svelte|ts)$/.test(entry.name)) files.push(path);
    }
  };
  walk('src');
  return files;
}

// Commands are always passed to `invoke` (directly or through a helper) as
// string literals, so every registered name in a literal is a used command.
function commandsIn(file) {
  const source = read(file);
  return [...new Set([...source.matchAll(/(['"`])([a-z][a-z0-9_]*)\1/g)].map((match) => match[2]))];
}

// The mini window mounts only this component (`src/main.ts`), so its commands
// are the literals reachable through its imports.
function miniWindowFiles() {
  const files = new Set();
  const visit = (file) => {
    if (files.has(file)) return;
    files.add(file);
    const source = read(file);
    for (const match of source.matchAll(/(?:from|import)\s+'(\.[^']+)'/g)) {
      const base = resolve(join(root, dirname(file)), match[1]);
      const target = ['', '.ts', '.svelte', '/index.ts'].map((suffix) => base + suffix)
        .find((candidate) => existsSync(candidate) && !candidate.endsWith('/'));
      assert.ok(target, `${file}: cannot resolve ${match[1]}`);
      visit(target.slice(root.length + 1));
    }
  };
  visit('src/lib/StudiumMini.svelte');
  return files;
}

const registered = registeredCommands();
const capabilityFiles = capabilities();
const miniFiles = miniWindowFiles();
const windows = {
  main: { files: sourceFiles().filter((file) => !miniFiles.has(file)), capability: 'default' },
  'studium-mini': { files: [...miniFiles], capability: 'studium-mini' },
};

function commandsByWindow(label) {
  return new Set(windows[label].files.flatMap(commandsIn).filter((name) => registered.includes(name)));
}

test('the app manifest declares exactly the registered commands', () => {
  const manifest = manifestCommands();
  assert.equal(new Set(manifest).size, manifest.length, 'duplicate command in the app manifest');
  assert.deepEqual([...manifest].sort(), [...registered].sort());
  assert.equal(new Set(registered).size, registered.length, 'duplicate command in generate_handler!');
});

test('capabilities only grant registered commands and known plugin permissions', () => {
  for (const capability of capabilityFiles) {
    for (const command of appCommands(capability)) {
      assert.ok(registered.includes(command), `${capability.file}: ${command} is not a registered command`);
    }
    for (const permission of pluginPermissions(capability)) {
      assert.match(permission, /^(core|dialog|opener|updater|autostart|single-instance):/,
        `${capability.file}: unexpected plugin permission ${permission}`);
    }
  }
});

test('each window grants exactly the commands its own code invokes', () => {
  for (const [label, { capability }] of Object.entries(windows)) {
    const granted = new Set(appCommands(capabilityFiles.find((entry) => entry.identifier === capability)));
    const used = commandsByWindow(label);
    assert.deepEqual([...used].filter((command) => !granted.has(command)).sort(), [],
      `${label}: invoked commands are missing from the ${capability} capability`);
    assert.deepEqual([...granted].filter((command) => !used.has(command)).sort(), [],
      `${label}: the ${capability} capability grants commands this window never invokes`);
  }
});

test('the mini window cannot reach the main window authority', () => {
  const mini = new Set(appCommands(capabilityFiles.find((entry) => entry.identifier === 'studium-mini')));
  for (const command of ['backup_restore', 'set_extension', 'save_settings', 'cybercampus_save_login',
    'ai_remote_key', 'archive_purge', 'ai_analyze']) {
    assert.ok(!mini.has(command), `studium-mini must not grant ${command}`);
  }
});
