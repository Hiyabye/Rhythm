import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync, execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { verifyArtifact, verifyManifest } from '../scripts/verify-update.mjs';

// Signatures are produced by the official Tauri signer with disposable keys that exist only in
// a temporary directory; the verifier under test is independent of that implementation.
// Invoke the JS entry point directly so Windows needs neither a POSIX shim nor
// shell quoting, and URL-escaped checkout paths are decoded on every host.
const tauri = fileURLToPath(new URL('../node_modules/@tauri-apps/cli/tauri.js', import.meta.url));
const repo = 'Hiyabye/Rhythm';
const version = '9.9.9';
const tag = `v${version}`;
const asset = 'Rhythm.app.tar.gz';
let root;
let publicKey;
let otherPublicKey;

function generate(name) {
  execFileSync(process.execPath, [tauri, 'signer', 'generate', '--ci', '-p', 'disposable', '-w', join(root, name), '-f'], { stdio: 'ignore', windowsHide: true });
  return readFileSync(join(root, `${name}.pub`), 'utf8').trim();
}

function sign(key, file, appVersion = version) {
  const result = spawnSync(process.execPath, [tauri, 'signer', 'sign', '-f', join(root, key), '--app-version', appVersion, file], {
    env: { ...process.env, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'disposable' }, encoding: 'utf8', windowsHide: true,
  });
  assert.equal(result.status, 0, result.stderr);
  return readFileSync(`${file}.sig`, 'utf8').trim();
}

function stage(name, bytes) {
  const dir = join(root, name);
  mkdirSync(dir);
  writeFileSync(join(dir, asset), bytes);
  return dir;
}

function manifestFor(signature) {
  const url = `https://github.com/${repo}/releases/download/${tag}/${asset}`;
  return {
    version, notes: `Rhythm ${version}`, pub_date: new Date().toISOString(),
    platforms: { 'darwin-aarch64': { url, signature }, 'darwin-aarch64-app': { url, signature } },
  };
}

before(() => {
  root = mkdtempSync(join(tmpdir(), 'rhythm-update-signature-'));
  publicKey = generate('release.key');
  otherPublicKey = generate('other.key');
});
after(() => rmSync(root, { recursive: true, force: true }));

const check = (dir, signature, overrides = {}) => verifyArtifact({
  bytes: readFileSync(join(dir, asset)), fileName: asset, signature, publicKey, version, ...overrides,
});

test('a matching artifact, signature and key verify', () => {
  const dir = stage('valid', 'genuine update bytes');
  const signature = sign('release.key', join(dir, asset));
  assert.equal(check(dir, signature).version, version);
  verifyManifest({ manifest: manifestFor(signature), assetsDir: dir, publicKey, version, repo, tag });
});

test('modified or corrupt artifacts are rejected', () => {
  const dir = stage('modified', 'genuine update bytes');
  const signature = sign('release.key', join(dir, asset));
  writeFileSync(join(dir, asset), 'genuine update bytez');
  assert.throws(() => check(dir, signature), /does not match/);
  writeFileSync(join(dir, asset), Buffer.alloc(0));
  assert.throws(() => check(dir, signature), /does not match/);
});

test('a signature from a different artifact is rejected', () => {
  const dir = stage('swapped', 'artifact A');
  const other = stage('other-artifact', 'artifact B');
  const signatureForB = sign('release.key', join(other, asset));
  assert.throws(() => check(dir, signatureForB), /does not match/);
});

test('a different or invalid public key is rejected', () => {
  const dir = stage('wrong-key', 'genuine update bytes');
  const signature = sign('release.key', join(dir, asset));
  assert.throws(() => check(dir, signature, { publicKey: otherPublicKey }), /different key|does not match/);
  assert.throws(() => check(dir, signature, { publicKey: '' }), /not Tauri base64/);
  assert.throws(() => check(dir, signature, { publicKey: Buffer.from('untrusted comment: x\nAAAA').toString('base64') }), /Public key/);
});

test('invalid, empty and non-empty garbage signatures are rejected', () => {
  const dir = stage('bad-signature', 'genuine update bytes');
  const signature = sign('release.key', join(dir, asset));
  // A non-empty .sig is not evidence of a signature: the old workflow accepted exactly this.
  assert.throws(() => check(dir, 'not a signature'), /not Tauri base64/);
  assert.throws(() => check(dir, Buffer.from('untrusted comment: x\nnonsense').toString('base64')), /Signature has 6 bytes/);
  assert.throws(() => check(dir, ''), /not Tauri base64/);
  const lines = Buffer.from(signature, 'base64').toString('utf8').split('\n');
  lines[2] = lines[2].replace(`version:${version}`, 'version:10.0.0');
  assert.throws(() => check(dir, Buffer.from(lines.join('\n')).toString('base64'), { version: '10.0.0' }), /not authenticated/);
});

test('a signature bound to another version or file name is rejected', () => {
  const dir = stage('version', 'genuine update bytes');
  const older = sign('release.key', join(dir, asset), '9.9.8');
  assert.throws(() => check(dir, older), /binds version 9.9.8/);
  const signature = sign('release.key', join(dir, asset));
  assert.throws(() => check(dir, signature, { fileName: 'Other.app.tar.gz' }), /made for Rhythm.app.tar.gz/);
});

test('incorrect manifest metadata is rejected', () => {
  const dir = stage('manifest', 'genuine update bytes');
  const signature = sign('release.key', join(dir, asset));
  const verifyWith = (manifest) => verifyManifest({ manifest, assetsDir: dir, publicKey, version, repo, tag });
  const mutate = (change) => { const manifest = manifestFor(signature); change(manifest); return manifest; };
  assert.throws(() => verifyWith(mutate((m) => { m.version = '9.9.10'; })), /Manifest version/);
  assert.throws(() => verifyWith(mutate((m) => { m.pub_date = 'yesterday'; })), /pub_date/);
  assert.throws(() => verifyWith(mutate((m) => { delete m.platforms['darwin-aarch64']; })), /exactly/);
  assert.throws(() => verifyWith(mutate((m) => { m.platforms['darwin-x86_64'] = m.platforms['darwin-aarch64']; })), /exactly/);
  assert.throws(() => verifyWith(mutate((m) => { m.platforms['darwin-aarch64'].url = `https://example.com/${asset}`; })), /outside/);
  assert.throws(() => verifyWith(mutate((m) => { m.platforms['darwin-aarch64'].signature = 'bm9uLWVtcHR5'; })), /differs/);
  rmSync(join(dir, `${asset}.sig`));
  assert.throws(() => verifyWith(manifestFor(signature)), /ENOENT/);
});

test('the release workflow verifies signatures cryptographically, not by presence', () => {
  const workflow = readFileSync(new URL('../.github/workflows/release-draft.yml', import.meta.url), 'utf8');
  // The previous gate accepted any non-empty .sig text.
  assert.doesNotMatch(workflow, /Empty updater signature|if \(!signature\)/);
  assert.match(workflow, /verify-update\.mjs artifact --artifact release-assets\/Rhythm\.app\.tar\.gz/);
  assert.match(workflow, /verify-update\.mjs bundle --archive release-assets\/Rhythm\.app\.tar\.gz/);
  assert.match(workflow, /verify-update\.mjs manifest --manifest release-assets\/latest\.json/);
  assert.match(workflow, /node scripts\/macos-release\.mjs --updater/);
  assert.match(workflow, /--draft/);
  assert.doesNotMatch(workflow, /gh release (edit|upload)|--draft=false|--latest/);
});
