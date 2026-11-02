import { test } from 'node:test';
import assert from 'node:assert/strict';
import { checkAssessment, checkSignature, preflight } from '../scripts/macos-release.mjs';

const identity = 'Developer ID Application: Validation Only (ABCDE12345)';
const signer = { identity, team: 'ABCDE12345' };
const keychain = `  1) 0123456789ABCDEF0123456789ABCDEF01234567 "${identity}"\n     1 valid identities found\n`;
const darwin = { platform: 'darwin', arch: 'arm64', identities: keychain };
const profile = { APPLE_SIGNING_IDENTITY: identity, RHYTHM_NOTARY_PROFILE: 'rhythm-notary' };

// Recorded from the v0.9.1 Release bundle built on this project's macOS host.
const adhoc = `Identifier=io.github.hiyabye.rhythm
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20500 size=39737 flags=0x10002(adhoc,runtime) hashes=1235+3 location=embedded
Signature=adhoc
TeamIdentifier=not set
Runtime Version=27.2.0`;
const developerId = `Identifier=io.github.hiyabye.rhythm
CodeDirectory v=20500 size=39737 flags=0x10000(runtime) hashes=1235+3 location=embedded
Signature size=9045
Authority=${identity}
Authority=Developer ID Certification Authority
Authority=Apple Root CA
Timestamp=2 Nov 2026 at 12:00:00
TeamIdentifier=ABCDE12345`;

test('missing or non-Developer ID identities never reach a build', () => {
  assert.throws(() => preflight({}, darwin), /not set; refusing/);
  assert.throws(() => preflight({ ...profile, APPLE_SIGNING_IDENTITY: '-' }, darwin), /Developer ID Application/);
  assert.throws(() => preflight({ ...profile, APPLE_SIGNING_IDENTITY: 'Apple Development: Someone (ABCDE12345)' }, darwin), /Developer ID Application/);
  assert.throws(() => preflight(profile, { ...darwin, identities: '     0 valid identities found\n' }), /not available in a keychain/);
  assert.throws(() => preflight(profile, { ...darwin, arch: 'x64' }), /Apple Silicon/);
});

test('notarization credentials are required and never echoed', () => {
  assert.throws(() => preflight({ APPLE_SIGNING_IDENTITY: identity }, darwin), /Notarization credentials are missing/);
  assert.throws(() => preflight({ APPLE_SIGNING_IDENTITY: identity, APPLE_API_KEY: 'KEY' }, darwin), /Notarization credentials are missing/);
  assert.deepEqual(preflight(profile, darwin).notary, ['--keychain-profile', 'rhythm-notary']);
  const api = preflight({ APPLE_SIGNING_IDENTITY: identity, APPLE_API_KEY: 'KEY', APPLE_API_ISSUER: 'ISSUER', APPLE_API_KEY_PATH: '/secure/key.p8' }, darwin);
  assert.deepEqual(api.notary, ['--key', '/secure/key.p8', '--key-id', 'KEY', '--issuer', 'ISSUER']);
});

test('ad-hoc, untimestamped or non-hardened signatures are rejected', () => {
  assert.throws(() => checkSignature(adhoc, signer, 'Rhythm.app'), /not signed by/);
  checkSignature(developerId, signer, 'Rhythm.app');
  assert.throws(() => checkSignature(developerId.replace(/^Timestamp=.*$/m, ''), signer, 'Rhythm.app'), /secure timestamp/);
  assert.throws(() => checkSignature(developerId.replace('0x10000(runtime)', '0x0(none)'), signer, 'Rhythm.app'), /hardened runtime/);
  checkSignature(developerId.replace('0x10000(runtime)', '0x0(none)'), signer, 'The DMG', { runtime: false });
  assert.throws(() => checkSignature(developerId.replace('TeamIdentifier=ABCDE12345', 'TeamIdentifier=ZZZZZ99999'), signer, 'Rhythm.app'), /team identifier/);
});

test('Gatekeeper must accept the artifact as notarized Developer ID code', () => {
  // The ad-hoc v0.9.1 app copied from its DMG produced exactly this assessment.
  assert.throws(() => checkAssessment('/tmp/Rhythm.app: rejected\n', 'the copied Rhythm.app'), /did not accept/);
  assert.throws(() => checkAssessment('/tmp/Rhythm.app: accepted\nsource=Developer ID\n', 'the copied Rhythm.app'), /did not accept/);
  checkAssessment('/tmp/Rhythm.app: accepted\nsource=Notarized Developer ID\norigin=Developer ID Application: Validation Only (ABCDE12345)\n', 'the copied Rhythm.app');
});
