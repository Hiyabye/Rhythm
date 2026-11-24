#!/usr/bin/env bash
# Prepares a Linux cloud agent checkout (Claude Code on the web through the
# SessionStart hook in .claude/settings.json, the Copilot coding agent through
# .github/workflows/copilot-setup-steps.yml). Linux is not a product platform:
# the app crate (src-tauri) needs macOS/Windows staging, so agents here run the
# frontend checks and the four library crates; CI covers the app on both OSes.
set -euo pipefail
cd "$(dirname "$0")/.."

# Claude Code runs every SessionStart hook locally too; only cloud sessions set up.
if [ "${1:-}" = "--claude-hook" ] && [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

npm ci --no-audit --no-fund
(cd worker && npm ci --ignore-scripts --no-audit --no-fund)

if command -v cargo >/dev/null; then
  minor=$(rustc --version | sed -E 's/^rustc 1\.([0-9]+).*/\1/')
  if [ "$minor" -lt 90 ] && command -v rustup >/dev/null; then
    rustup toolchain install stable --profile minimal --component clippy,rustfmt
    rustup default stable
  fi
  cargo fetch --locked
else
  echo "agent-setup: cargo not found; Rust checks are unavailable" >&2
fi
