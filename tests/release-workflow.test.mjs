import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('the tag release workflow creates a draft from distribution files only', () => {
  const workflow = read('.github/workflows/release.yml');
  assert.match(workflow, /tags: \['v\*\.\*\.\*'\]/);
  assert.match(workflow, /merge-base --is-ancestor/);
  assert.match(workflow, /refusing to overwrite/);
  assert.match(workflow, /gh release create "\$RELEASE_TAG" release-assets\/\* --draft --verify-tag/);
  assert.doesNotMatch(workflow, /gh release (edit|upload)|--draft=false|--latest/);
  assert.doesNotMatch(workflow, /release\/bundle/);
  assert.equal(workflow.match(/path: target\/release\/distribution\/\*/g)?.length, 2);
  // Unsigned builds only: no signing or updater secrets reach this workflow.
  assert.doesNotMatch(workflow, /secrets\.|TAURI_SIGNING|APPLE_/);
  // Write access only in the job that creates the release.
  assert.equal(workflow.match(/contents: write/g)?.length, 1);
  assert.match(workflow, /draft-release:[\s\S]*permissions:\s*\n\s*contents: write/);
  // The DMG must never ship a CPU-only worker (scripts/desktop.mjs fails instead).
  assert.match(workflow, /RHYTHM_AI_BACKEND: metal\s*\n\s*run: npm run desktop:build/);
});

test('CI and the release share the pinned Windows build tools', () => {
  for (const path of ['.github/workflows/ci.yml', '.github/workflows/release.yml']) {
    const workflow = read(path);
    assert.match(workflow, /uses: \.\/\.github\/actions\/windows-build-tools/, path);
    assert.doesNotMatch(workflow, /vulkansdk-windows|LLVM-.*-win64/, path);
  }
});

test('native CI uses verified worker artifacts and tests their staged executables', () => {
  const workflow = read('.github/workflows/ci.yml');
  assert.equal(workflow.match(/npm run worker:build -- --debug/g)?.length, 2);
  assert.doesNotMatch(workflow, /cargo build --locked -p rhythm-ai-worker|Copy-Item .*rhythm-ai-worker|fs\.copyFileSync/);
  assert.match(workflow, /RHYTHM_WORKER_TEST_BIN=\$\(\(Resolve-Path src-tauri\/binaries\/rhythm-ai-worker-x86_64-pc-windows-msvc\.exe/);
  assert.match(workflow, /p\.resolve\("src-tauri\/binaries\/rhythm-ai-worker-"\+target\)/);
});
