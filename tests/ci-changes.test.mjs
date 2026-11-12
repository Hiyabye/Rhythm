import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { classify } from '../scripts/ci-changes.mjs';

const none = { frontend: false, rust: false, packaging: false };
const all = { frontend: true, rust: true, packaging: true };

test('documentation-only changes run no jobs', () => {
  assert.deepEqual(classify(['README.md', 'AGENTS.md', 'docs/ROADMAP.md', 'PRODUCT.md']), none);
});

test('markdown never runs the Rust or packaging jobs', () => {
  assert.deepEqual(classify(['src-tauri/src/archive/certificates/README.md', 'src-tauri/src/archive/fixtures/README.md']), none);
  // Still a frontend change: tests/ and scripts/ hold Node test inputs.
  assert.deepEqual(classify(['scripts/NOTES.md']), { ...none, frontend: true });
  // A real Rust change in the same diff still runs Rust.
  assert.deepEqual(classify(['src-tauri/src/archive/fixtures/README.md', 'src-tauri/src/main.rs']), { ...none, frontend: true, rust: true });
});

test('each area selects only its own jobs', () => {
  assert.deepEqual(classify(['src/App.svelte', 'tests/ui/archive.spec.ts']), { ...none, frontend: true });
  assert.deepEqual(classify(['playwright.screenshots.config.ts']), { ...none, frontend: true });
  assert.deepEqual(classify(['src-tauri/src/main.rs']), { ...none, frontend: true, rust: true });
  assert.deepEqual(classify(['src-tauri/vendor/mac-usernotifications/src/lib.rs']), { ...none, rust: true });
  assert.deepEqual(classify(['src-tauri/vendor/mac-usernotifications/Cargo.toml']), { ...none, rust: true });
  assert.deepEqual(classify(['Cargo.lock']), { ...none, rust: true, packaging: true });
  assert.deepEqual(classify(['scripts/desktop.mjs']), { ...none, frontend: true, packaging: true });
  assert.deepEqual(classify(['src-tauri/tauri.windows.conf.json']), { ...none, rust: true, packaging: true });
  assert.deepEqual(classify(['LICENSE']), { ...none, packaging: true });
});

test('frontend files read by Rust tests also run the Rust jobs', () => {
  for (const file of ['src/lib/analysis-contract.json', 'src/lib/UpdaterSettings.svelte']) {
    assert.deepEqual(classify([file]), { ...none, frontend: true, rust: true }, file);
  }
  // Windows CI runs its PowerShell scripts before the Rust tests.
  assert.equal(classify(['scripts/stage-vc-runtime.ps1']).rust, true);
  for (const file of ['scripts/stage-llama.mjs', 'scripts/llama-cpp.json']) {
    assert.deepEqual(classify([file]), { ...none, frontend: true, rust: true, packaging: true }, file);
  }
});

test('the command permission check runs for every file it reads', () => {
  // tests/capabilities.test.mjs compares the registered commands, the app
  // manifest and the capability files, so a change to any of them must run the
  // frontend job that holds the check.
  for (const file of ['src-tauri/build.rs', 'src-tauri/src/main.rs', 'src-tauri/capabilities/default.json',
    'src-tauri/capabilities/studium-mini.json']) {
    assert.equal(classify([file]).frontend, true, file);
  }
  // Other Rust paths keep the Rust-only behavior.
  assert.equal(classify(['src-tauri/src/commands.rs']).frontend, false);
});

test('workflow changes and unknown change sets run everything', () => {
  assert.deepEqual(classify(['.github/workflows/ci.yml']), all);
  assert.deepEqual(classify(['.github/actions/windows-build-tools/action.yml']), all);
  assert.deepEqual(classify(null), all);
});

test('pull requests never build packages', () => {
  assert.deepEqual(classify(['Cargo.lock'], { pullRequest: true }), { ...none, rust: true });
  assert.deepEqual(classify(null, { pullRequest: true }), { ...all, packaging: false });
});

const script = join(dirname(fileURLToPath(import.meta.url)), '..', 'scripts', 'ci-changes.mjs');
function run(cwd, env) {
  const output = join(cwd, 'github-output');
  writeFileSync(output, '');
  execFileSync(process.execPath, [script], { cwd, env: { ...process.env, GITHUB_OUTPUT: output, ...env }, stdio: 'pipe' });
  return Object.fromEntries(readFileSync(output, 'utf8').trim().split('\n').map((line) => {
    const [name, value] = line.split('=');
    return [name, value === 'true'];
  }));
}

test('pull requests diff the merge commit against the current base, not older base history', () => {
  const root = mkdtempSync(join(tmpdir(), 'rhythm-ci-changes-'));
  try {
    const git = (...args) => execFileSync('git', ['-c', 'user.name=t', '-c', 'user.email=t@example.com', ...args], { cwd: root, stdio: 'pipe', encoding: 'utf8' });
    const commit = (file, message) => {
      mkdirSync(dirname(join(root, file)), { recursive: true });
      writeFileSync(join(root, file), message);
      git('add', '.');
      git('commit', '-qm', message);
    };
    git('init', '-q', '-b', 'main');
    commit('README.md', 'base');
    git('checkout', '-qb', 'feature');
    commit('src/App.svelte', 'feature');
    git('checkout', '-q', 'main');
    // Lands on main after the branch point; must not count as part of the PR.
    commit('src-tauri/src/main.rs', 'other merge');
    git('merge', '-q', '--no-ff', '-m', 'merge', 'feature');
    assert.deepEqual(run(root, { GITHUB_EVENT_NAME: 'pull_request' }), { ...none, frontend: true });
    // Without a known previous commit, every job runs.
    assert.deepEqual(run(root, { GITHUB_EVENT_NAME: 'push', CI_BEFORE: '0'.repeat(40) }), all);
    assert.deepEqual(run(root, { GITHUB_EVENT_NAME: 'workflow_dispatch' }), all);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
