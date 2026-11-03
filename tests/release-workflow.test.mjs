import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';

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
  // No OS code signing: Apple credentials never reach this workflow.
  assert.doesNotMatch(workflow, /APPLE_|codesign|notarytool|signtool/);
  // Updater signing secrets reach only the two build steps, in jobs gated
  // by the reviewer-approved `release` environment.
  assert.equal(workflow.match(/secrets\.TAURI_SIGNING_PRIVATE_KEY }}/g)?.length, 2);
  assert.equal(workflow.match(/secrets\.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}/g)?.length, 2);
  assert.equal(workflow.match(/secrets\./g)?.length, 4);
  assert.equal(workflow.match(/environment: release/g)?.length, 2);
  for (const job of ['desktop', 'windows']) {
    assert.match(workflow, new RegExp(`\\n  ${job}:\\n(    .*\\n)*?    environment: release\\n`), job);
  }
  // Missing secrets fail the release instead of dropping updater files.
  assert.equal(workflow.match(/TAURI_SIGNING_PRIVATE_KEY is not configured/g)?.length, 2);
  // Write access only in the job that creates the release.
  assert.equal(workflow.match(/contents: write/g)?.length, 1);
  assert.match(workflow, /draft-release:[\s\S]*permissions:\s*\n\s*contents: write/);
  // The DMG bundles the pinned llama.cpp build; no worker backend is compiled.
  assert.match(workflow, /name: Build DMG and signed updater archive\n[\s\S]*?npm run desktop:build/);
  assert.doesNotMatch(workflow, /RHYTHM_AI_BACKEND|metaltoolchain|target-gpu/);
});

test('CI and the release share the Windows build tools, which install no GPU toolchain', () => {
  for (const path of ['.github/workflows/ci.yml', '.github/workflows/release.yml']) {
    const workflow = read(path);
    assert.match(workflow, /uses: \.\/\.github\/actions\/windows-build-tools/, path);
    assert.doesNotMatch(workflow, /vulkansdk-windows|LLVM-.*-win64|CMAKE_|crates\/ai-worker/, path);
  }
  assert.doesNotMatch(read('.github/actions/windows-build-tools/action.yml'), /vulkan|llvm|libclang|ninja/i);
});

test('release.yml is the only publishing workflow', () => {
  const workflows = readdirSync(new URL('../.github/workflows/', import.meta.url)).sort();
  assert.deepEqual(workflows, ['ci.yml', 'release.yml']);
});
