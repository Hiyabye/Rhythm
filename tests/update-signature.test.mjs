import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync, execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { checkEmbeddedKey, verifyArtifact, verifyManifest } from '../scripts/verify-update.mjs';
import { buildManifest, parseArguments } from '../scripts/update-manifest.mjs';

// Signatures are produced by the official Tauri signer with disposable keys that exist only in
// a temporary directory; the verifier under test is independent of that implementation.
// Invoke the JS entry point directly so Windows needs neither a POSIX shim nor
// shell quoting, and URL-escaped checkout paths are decoded on every host.
const tauri = fileURLToPath(new URL('../node_modules/@tauri-apps/cli/tauri.js', import.meta.url));
const repo = 'Hiyabye/Rhythm';
const version = '9.9.9';
const tag = `v${version}`;
const asset = 'Rhythm.app.tar.gz';
const installer = `Rhythm_${version}_x64-setup.exe`;
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

// Both platforms' signed packages, staged as the release workflow does.
function release(name) {
  const dir = stage(name, 'genuine update bytes');
  writeFileSync(join(dir, installer), 'genuine installer bytes');
  sign('release.key', join(dir, asset));
  sign('release.key', join(dir, installer));
  return dir;
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
});

test('the generated manifest covers both platforms and verifies', () => {
  const dir = release('generated');
  const manifest = buildManifest({ assetsDir: dir, version, repo, tag, now: new Date('2026-11-04T00:00:00Z') });
  const base = `https://github.com/${repo}/releases/download/${tag}/`;
  assert.deepEqual(Object.keys(manifest.platforms).sort(), ['darwin-aarch64', 'darwin-aarch64-app', 'windows-x86_64', 'windows-x86_64-nsis']);
  assert.equal(manifest.platforms['darwin-aarch64-app'].url, base + asset);
  assert.equal(manifest.platforms['windows-x86_64-nsis'].url, base + installer);
  assert.equal(manifest.platforms['windows-x86_64'].signature, readFileSync(join(dir, `${installer}.sig`), 'utf8').trim());
  assert.equal(manifest.pub_date, '2026-11-04T00:00:00.000Z');
  verifyManifest({ manifest, assetsDir: dir, publicKey, version, repo, tag });
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
  const dir = release('manifest');
  const manifestFor = () => buildManifest({ assetsDir: dir, version, repo, tag });
  const verifyWith = (manifest) => verifyManifest({ manifest, assetsDir: dir, publicKey, version, repo, tag });
  const mutate = (change) => { const manifest = manifestFor(); change(manifest); return manifest; };
  assert.throws(() => verifyWith(mutate((m) => { m.version = '9.9.10'; })), /Manifest version/);
  assert.throws(() => verifyWith(mutate((m) => { m.pub_date = 'yesterday'; })), /pub_date/);
  assert.throws(() => verifyWith(mutate((m) => { delete m.platforms['darwin-aarch64']; })), /exactly/);
  assert.throws(() => verifyWith(mutate((m) => { m.platforms['darwin-x86_64'] = m.platforms['darwin-aarch64']; })), /exactly/);
  assert.throws(() => verifyWith(mutate((m) => { m.platforms['darwin-aarch64'].url = `https://example.com/${asset}`; })), /outside/);
  assert.throws(() => verifyWith(mutate((m) => { m.platforms['darwin-aarch64'].signature = 'bm9uLWVtcHR5'; })), /differs/);
  assert.throws(() => verifyWith(mutate((m) => { delete m.platforms['windows-x86_64-nsis']; })), /exactly/);
  // Each platform must point at its own package, never the other platform's.
  assert.throws(() => verifyWith(mutate((m) => { m.platforms['windows-x86_64'] = m.platforms['darwin-aarch64']; })), /its updater package/);
  assert.throws(() => verifyWith(mutate((m) => { m.platforms['darwin-aarch64-app'] = m.platforms['windows-x86_64']; })), /its updater package/);
  // A Windows signature swapped in from the macOS archive does not verify.
  assert.throws(() => verifyWith(mutate((m) => { m.platforms['windows-x86_64-nsis'].signature = m.platforms['darwin-aarch64'].signature; })), /differs/);
  const manifest = manifestFor();
  writeFileSync(join(dir, installer), 'tampered installer bytes');
  assert.throws(() => verifyWith(manifest), /does not match Rhythm_9\.9\.9_x64-setup\.exe/);
  rmSync(join(dir, `${installer}.sig`));
  assert.throws(() => manifestFor(), /ENOENT/);
});

test('the shipped binary must embed the configured public key', () => {
  const binary = join(root, 'rhythm.exe');
  writeFileSync(binary, Buffer.concat([Buffer.from([0, 1, 2]), Buffer.from(publicKey), Buffer.from([3])]));
  checkEmbeddedKey(binary, publicKey);
  assert.throws(() => checkEmbeddedKey(binary, otherPublicKey), /does not embed/);
});

test('the release workflow verifies signatures cryptographically, not by presence', () => {
  const workflow = readFileSync(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8');
  assert.match(workflow, /verify-update\.mjs artifact --artifact target\/release\/distribution\/Rhythm\.app\.tar\.gz/);
  assert.match(workflow, /verify-update\.mjs bundle --archive target\/release\/distribution\/Rhythm\.app\.tar\.gz/);
  assert.match(workflow, /verify-update\.mjs artifact --artifact "target\/release\/distribution\/Rhythm_\$\(\$env:RELEASE_VERSION\)_x64-setup\.exe"/);
  assert.match(workflow, /verify-update\.mjs binary --file target\/release\/rhythm\.exe/);
  // The manifest is generated and verified before the draft is created.
  const generate = workflow.indexOf('update-manifest.mjs');
  const verify = workflow.indexOf('verify-update.mjs manifest --manifest release-assets/latest.json');
  const create = workflow.indexOf('gh release create');
  assert.ok(generate > 0 && generate < verify && verify < create);
});

test('manifest generator names every missing or malformed option', () => {
  assert.throws(() => parseArguments(['--assets', 'a', '--version', '1.0.0']), /Missing --repo, --tag, --output/);
  assert.throws(() => parseArguments(['--assets']), /Usage: update-manifest/);
  assert.throws(() => parseArguments(['assets', 'a']), /Usage: update-manifest/);
  const all = ['--assets', 'a', '--version', '1.0.0', '--repo', 'o/n', '--tag', 'v1.0.0', '--output', 'f'];
  assert.equal(parseArguments(all).tag, 'v1.0.0');
});
