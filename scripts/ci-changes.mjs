import { execFileSync } from 'node:child_process';
import { appendFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Decides which CI jobs a change needs. A path missing from a rule silently
// skips its checks, so rules err toward running, and any workflow or action
// change runs everything.
const workflow = /^\.github\//;
const rules = {
  frontend: [/^src\//, /^tests\//, /^scripts\//, /^package(-lock)?\.json$/, /^index\.html$/,
    /^vite\.config\.ts$/, /^tsconfig\.json$/, /^playwright(\.screenshots)?\.config\.ts$/],
  rust: [/^src-tauri\//, /^Cargo\.(toml|lock)$/, /^\.cargo\//, /^rust-toolchain/,
    /^(rustfmt|clippy)\.toml$/,
    // Windows CI runs these PowerShell scripts before its Rust tests.
    /^scripts\/[^/]+\.ps1$/,
    // Rust CI stages the pinned llama.cpp build; the real-server test runs it.
    /^scripts\/(stage-llama\.mjs|llama-cpp\.json)$/,
    // Rust tests read these through include_str!.
    /^src\/lib\/analysis-contract\.json$/, /^src\/lib\/UpdaterSettings\.svelte$/],
  // Release builds run on main only; a pull request never builds packages.
  packaging: [/^scripts\//, /^src-tauri\/(tauri[^/]*\.conf\.json|build\.rs|Cargo\.toml)$/,
    /^src-tauri\/(icons|capabilities)\//, /^Cargo\.(toml|lock)$/,
    /^package(-lock)?\.json$/],
};

// Markdown under src-tauri/ or scripts/ is documentation; no Rust
// build or test reads it.
const markdown = /\.md$/i;
const ignoresMarkdown = new Set(['rust', 'packaging']);

// `files` is null when the changed set is unknown, which runs everything.
export function classify(files, { pullRequest = false } = {}) {
  const all = files === null || files.some((file) => workflow.test(file));
  const flags = Object.fromEntries(Object.entries(rules).map(([name, patterns]) =>
    [name, all || files.some((file) => !(ignoresMarkdown.has(name) && markdown.test(file))
      && patterns.some((pattern) => pattern.test(file)))]));
  if (pullRequest) flags.packaging = false;
  return flags;
}

// Changed files for this event, or null when they cannot be determined.
function changedFiles({ event, before }) {
  const git = (...args) => execFileSync('git', args, { encoding: 'utf8' });
  // A pull request checks out its merge commit; the first parent is the base tip.
  if (event === 'pull_request') return git('diff', '--name-only', 'HEAD^1', 'HEAD').split('\n').filter(Boolean);
  if (event !== 'push' || !before || /^0+$/.test(before)) return null;
  try {
    git('fetch', '--quiet', '--depth=1', 'origin', before);
  } catch {
    return null; // force-pushed or otherwise unreachable
  }
  return git('diff', '--name-only', before, 'HEAD').split('\n').filter(Boolean);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const event = process.env.GITHUB_EVENT_NAME;
  const files = changedFiles({ event, before: process.env.CI_BEFORE });
  console.log(files === null ? 'Changed files unknown; running every job.' : `Changed files:\n${files.join('\n')}`);
  const flags = classify(files, { pullRequest: event === 'pull_request' });
  console.log(flags);
  appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(flags).map(([name, value]) => `${name}=${value}\n`).join(''));
}
