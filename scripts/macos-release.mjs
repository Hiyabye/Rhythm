// Developer ID release path: build, sign, notarize, staple and verify the app and DMG.
// It never falls back to ad-hoc signing; missing credentials stop it before any build.
//
//   APPLE_SIGNING_IDENTITY="Developer ID Application: Name (TEAMID)"
//   RHYTHM_NOTARY_PROFILE=<notarytool keychain profile>          (local)
//   or APPLE_API_KEY + APPLE_API_ISSUER + APPLE_API_KEY_PATH     (CI)
//
//   node scripts/macos-release.mjs [--updater]   build and publish-ready artifacts
//   node scripts/macos-release.mjs --verify <dmg>  verify an existing DMG only
import { spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const IDENTITY = /^Developer ID Application: .+ \(([A-Z0-9]{10})\)$/;

function run(command, args, { env, capture = false } = {}) {
  const result = spawnSync(command, args, { env: env ?? process.env, encoding: 'utf8', stdio: capture ? 'pipe' : 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args[0]} failed (${result.status})${capture ? `:\n${result.stderr || result.stdout}` : ''}`);
  }
  // codesign and spctl report on stderr.
  return capture ? `${result.stdout}${result.stderr}` : '';
}

/** The expected signer; ad-hoc and development identities cannot be notarized. */
export function expectedSigner(env = process.env) {
  const identity = env.APPLE_SIGNING_IDENTITY?.trim();
  if (!identity) throw new Error('APPLE_SIGNING_IDENTITY is not set; refusing to produce or accept an unsigned or ad-hoc release.');
  const match = IDENTITY.exec(identity);
  if (!match) throw new Error('APPLE_SIGNING_IDENTITY must be a "Developer ID Application: Name (TEAMID)" identity; ad-hoc and development identities cannot be notarized.');
  return { identity, team: match[1] };
}

/** Validates credentials without printing them; returns what the release needs. */
export function preflight(env = process.env, { platform = process.platform, arch = process.arch, identities } = {}) {
  if (platform !== 'darwin' || arch !== 'arm64') throw new Error('Developer ID releases are built on Apple Silicon macOS only.');
  const { identity, team } = expectedSigner(env);
  const available = identities ?? run('security', ['find-identity', '-v', '-p', 'codesigning'], { capture: true });
  if (!available.includes(`"${identity}"`)) throw new Error('The Developer ID identity is not available in a keychain with its private key.');
  let notary;
  if (env.RHYTHM_NOTARY_PROFILE?.trim()) {
    notary = ['--keychain-profile', env.RHYTHM_NOTARY_PROFILE.trim()];
  } else if (env.APPLE_API_KEY && env.APPLE_API_ISSUER && env.APPLE_API_KEY_PATH) {
    notary = ['--key', env.APPLE_API_KEY_PATH, '--key-id', env.APPLE_API_KEY, '--issuer', env.APPLE_API_ISSUER];
  } else {
    throw new Error('Notarization credentials are missing: set RHYTHM_NOTARY_PROFILE (xcrun notarytool store-credentials) or APPLE_API_KEY, APPLE_API_ISSUER and APPLE_API_KEY_PATH.');
  }
  return { identity, team, notary };
}

/** Parses `codesign -dvv` output and rejects anything but a timestamped Developer ID hardened-runtime signature. */
export function checkSignature(details, { identity, team }, what, { runtime = true } = {}) {
  const authority = /^Authority=(.+)$/m.exec(details)?.[1];
  if (/^Signature=adhoc$/m.test(details) || authority !== identity) throw new Error(`${what} is not signed by ${identity}.`);
  if (/^TeamIdentifier=(.+)$/m.exec(details)?.[1] !== team) throw new Error(`${what} has the wrong team identifier.`);
  if (runtime && !/^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*\bruntime\b[^)]*\)/m.test(details)) throw new Error(`${what} lacks the hardened runtime.`);
  if (!/^Timestamp=/m.test(details)) throw new Error(`${what} lacks a secure timestamp.`);
}

/** Gatekeeper must attribute the artifact to notarized Developer ID code. */
export function checkAssessment(output, what) {
  if (!/: accepted$/m.test(output) || !/^source=Notarized Developer ID$/m.test(output)) {
    throw new Error(`Gatekeeper did not accept ${what} as notarized Developer ID code:\n${output.trim()}`);
  }
}

function verifyApp(app, signer) {
  run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', app], { capture: true });
  checkSignature(run('codesign', ['-dvv', app], { capture: true }), signer, 'Rhythm.app');
  checkSignature(run('codesign', ['-dvv', join(app, 'Contents/MacOS/rhythm-ai-worker')], { capture: true }), signer, 'The bundled AI worker');
  const entitlements = run('codesign', ['-d', '--entitlements', '-', '--xml', app], { capture: true });
  if (entitlements.includes('com.apple.security.get-task-allow')) throw new Error('Rhythm.app carries the debugging entitlement get-task-allow.');
  run('xcrun', ['stapler', 'validate', app], { capture: true });
}

function notarize(file, notary) {
  const output = run('xcrun', ['notarytool', 'submit', file, ...notary, '--wait', '--output-format', 'json'], { capture: true });
  const result = JSON.parse(output.slice(output.indexOf('{'), output.lastIndexOf('}') + 1));
  if (result.status !== 'Accepted') {
    const log = result.id ? run('xcrun', ['notarytool', 'log', result.id, ...notary], { capture: true }) : '';
    throw new Error(`Notarization of ${file} ended with ${result.status}.\n${log}`);
  }
  run('xcrun', ['stapler', 'staple', file]);
}

/** Verifies a DMG as a user receives it, including the app copied out of it. */
export function verifyDmg(dmg, signer) {
  run('hdiutil', ['verify', dmg], { capture: true });
  // A disk image carries no executable code, so the hardened runtime does not apply.
  checkSignature(run('codesign', ['-dvv', dmg], { capture: true }), signer, 'The DMG', { runtime: false });
  run('xcrun', ['stapler', 'validate', dmg], { capture: true });
  checkAssessment(run('spctl', ['-a', '-t', 'open', '--context', 'context:primary-signature', '-vv', dmg], { capture: true }), 'the DMG');
  const work = mkdtempSync(join(tmpdir(), 'rhythm-release-verify-'));
  const mount = join(work, 'mount');
  mkdirSync(mount);
  try {
    run('hdiutil', ['attach', '-readonly', '-nobrowse', '-noautoopen', '-mountpoint', mount, dmg], { capture: true });
    try {
      verifyApp(join(mount, 'Rhythm.app'), signer);
      const installed = join(work, 'Applications', 'Rhythm.app');
      run('ditto', [join(mount, 'Rhythm.app'), installed]);
      verifyApp(installed, signer);
      checkAssessment(run('spctl', ['-a', '-t', 'exec', '-vv', installed], { capture: true }), 'the copied Rhythm.app');
    } finally {
      run('hdiutil', ['detach', mount, '-quiet']);
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

function release({ updater }) {
  const signer = preflight();
  const version = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8')).version;
  // Tauri signs the app and its nested worker inside-out with the hardened runtime.
  // Notarization is done once below, so Tauri's own notarization credentials are withheld.
  const env = { ...process.env };
  for (const name of ['APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID', 'APPLE_API_KEY', 'APPLE_API_ISSUER', 'APPLE_API_KEY_PATH', 'APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD']) delete env[name];
  const config = { bundle: { macOS: { signingIdentity: signer.identity } } };
  if (updater) {
    // The Tauri CLI checks its updater signature against the configured key, and the
    // committed config deliberately has none; the same key is compiled in via option_env!.
    const pubkey = process.env.RHYTHM_UPDATER_PUBKEY?.trim();
    if (!pubkey || !process.env.TAURI_SIGNING_PRIVATE_KEY) throw new Error('Updater artifacts need TAURI_SIGNING_PRIVATE_KEY and RHYTHM_UPDATER_PUBKEY.');
    config.bundle.createUpdaterArtifacts = true;
    config.plugins = { updater: { pubkey } };
  }
  run('node', ['scripts/desktop.mjs', 'build', '--bundles', 'app', '--config', JSON.stringify(config)], { env });

  const bundle = resolve('target/release/bundle');
  const app = join(bundle, 'macos/Rhythm.app');
  run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', app], { capture: true });
  checkSignature(run('codesign', ['-dvv', app], { capture: true }), signer, 'Rhythm.app');
  const zip = join(bundle, 'macos/Rhythm-notarization.zip');
  run('ditto', ['-c', '-k', '--keepParent', app, zip]);
  notarize(zip, signer.notary);
  rmSync(zip);
  // Staple the app before it goes into the DMG so the copied app carries its ticket offline.
  run('xcrun', ['stapler', 'staple', app]);
  verifyApp(app, signer);

  const output = join(bundle, 'notarized');
  const staging = join(output, 'dmg-root');
  rmSync(output, { recursive: true, force: true });
  mkdirSync(staging, { recursive: true });
  run('ditto', [app, join(staging, 'Rhythm.app')]);
  symlinkSync('/Applications', join(staging, 'Applications'));
  const dmg = join(output, `Rhythm_${version}_aarch64.dmg`);
  run('hdiutil', ['create', '-volname', 'Rhythm', '-srcfolder', staging, '-fs', 'HFS+', '-format', 'UDZO', dmg]);
  rmSync(staging, { recursive: true });
  run('codesign', ['--sign', signer.identity, '--timestamp', dmg]);
  notarize(dmg, signer.notary);
  verifyDmg(dmg, signer);
  if (updater) {
    // Tauri archived the signed app before stapling; the updater installs it without
    // quarantine and Gatekeeper can still find its ticket online by code hash.
    for (const name of ['Rhythm.app.tar.gz', 'Rhythm.app.tar.gz.sig']) cpSync(join(bundle, 'macos', name), join(output, name));
  }
  console.log(`Developer ID signed, notarized and stapled: ${dmg}`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  try {
    const args = process.argv.slice(2);
    if (args[0] === '--verify') {
      if (!args[1]) throw new Error('Usage: macos-release.mjs --verify <dmg>');
      verifyDmg(resolve(args[1]), expectedSigner());
      console.log(`Verified ${args[1]}`);
    } else {
      release({ updater: args.includes('--updater') });
    }
  } catch (error) {
    console.error(`macOS release failed: ${error.message}`);
    process.exit(1);
  }
}
