// Independent verifier for Tauri updater signatures and the stable `latest.json` manifest.
// Tauri wraps minisign key/signature files in one extra base64 layer; the updater accepts a
// package only if Ed25519 verifies BLAKE2b-512(package) and the global signature covers the
// trusted comment that binds `version:`. This mirrors those checks without dependencies so the
// release workflow cannot pass merely because a `.sig` file is non-empty.
import { execFileSync } from 'node:child_process';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { pathToFileURL } from 'node:url';

export const UPDATER_TARGETS = ['darwin-aarch64', 'darwin-aarch64-app'];

function unwrap(value, what) {
  const text = String(value ?? '').trim();
  if (!text || !/^[A-Za-z0-9+/]+={0,2}$/.test(text)) throw new Error(`${what} is not Tauri base64.`);
  return Buffer.from(text, 'base64').toString('utf8');
}

function decodeLine(line, length, what) {
  if (!line || !/^[A-Za-z0-9+/]+={0,2}$/.test(line)) throw new Error(`${what} is malformed.`);
  const bytes = Buffer.from(line, 'base64');
  if (bytes.length !== length) throw new Error(`${what} has ${bytes.length} bytes, expected ${length}.`);
  return bytes;
}

export function parsePublicKey(value) {
  const lines = unwrap(value, 'Public key').split('\n').map((line) => line.trimEnd());
  if (!lines[0]?.startsWith('untrusted comment:')) throw new Error('Public key lacks its minisign comment.');
  const bytes = decodeLine(lines[1], 42, 'Public key');
  if (bytes.subarray(0, 2).toString('latin1') !== 'Ed') throw new Error('Public key is not an Ed25519 minisign key.');
  return {
    keyId: bytes.subarray(2, 10),
    key: createPublicKey({ key: { kty: 'OKP', crv: 'Ed25519', x: bytes.subarray(10).toString('base64url') }, format: 'jwk' }),
  };
}

export function parseSignature(value) {
  const lines = unwrap(value, 'Signature').split('\n').map((line) => line.trimEnd());
  if (!lines[0]?.startsWith('untrusted comment:')) throw new Error('Signature lacks its minisign comment.');
  const bytes = decodeLine(lines[1], 74, 'Signature');
  // Tauri signs BLAKE2b-512 prehashed packages ("ED"); legacy unhashed signatures are refused.
  if (bytes.subarray(0, 2).toString('latin1') !== 'ED') throw new Error('Signature is not a prehashed Ed25519 minisign signature.');
  const prefix = 'trusted comment: ';
  if (!lines[2]?.startsWith(prefix)) throw new Error('Signature lacks its trusted comment.');
  const trustedComment = lines[2].slice(prefix.length);
  return {
    keyId: bytes.subarray(2, 10),
    signature: bytes.subarray(10),
    trustedComment,
    globalSignature: decodeLine(lines[3], 64, 'Global signature'),
  };
}

/** Verify a package against its Tauri signature; returns the authenticated trusted-comment fields. */
export function verifyArtifact({ bytes, fileName, signature, publicKey, version }) {
  const key = parsePublicKey(publicKey);
  const sig = parseSignature(signature);
  if (!key.keyId.equals(sig.keyId)) throw new Error('Signature was made by a different key.');
  const digest = createHash('blake2b512').update(bytes).digest();
  if (!verify(null, digest, key.key, sig.signature)) throw new Error(`Signature does not match ${fileName}.`);
  const global = Buffer.concat([sig.signature, Buffer.from(sig.trustedComment, 'utf8')]);
  if (!verify(null, global, key.key, sig.globalSignature)) throw new Error('Trusted comment is not authenticated.');
  const fields = Object.fromEntries(sig.trustedComment.split('\t').map((field) => {
    const at = field.indexOf(':');
    return at < 0 ? [field, ''] : [field.slice(0, at), field.slice(at + 1)];
  }));
  if (fields.file !== fileName) throw new Error(`Signature was made for ${fields.file ?? 'an unnamed file'}, not ${fileName}.`);
  if (fields.version !== version) throw new Error(`Signature binds version ${fields.version ?? '(none)'}, expected ${version}.`);
  return fields;
}

/** Validate the stable manifest exactly as published, using the staged assets it references. */
export function verifyManifest({ manifest, assetsDir, publicKey, version, repo, tag }) {
  if (manifest?.version !== version) throw new Error(`Manifest version ${manifest?.version} does not match ${version}.`);
  if (Number.isNaN(Date.parse(manifest.pub_date ?? ''))) throw new Error('Manifest pub_date is not a valid date.');
  const targets = Object.keys(manifest.platforms ?? {}).sort();
  if (targets.join() !== UPDATER_TARGETS.join()) throw new Error(`Manifest targets must be exactly ${UPDATER_TARGETS.join(', ')}; found ${targets.join(', ') || 'none'}.`);
  const prefix = `https://github.com/${repo}/releases/download/${tag}/`;
  for (const target of targets) {
    const { url, signature } = manifest.platforms[target];
    if (typeof url !== 'string' || !url.startsWith(prefix)) throw new Error(`${target} URL is outside ${prefix}.`);
    const asset = decodeURIComponent(url.slice(prefix.length));
    if (!asset || asset.includes('/') || !asset.endsWith('.app.tar.gz')) throw new Error(`${target} does not reference a macOS updater archive.`);
    const sigFile = readFileSync(join(assetsDir, `${asset}.sig`), 'utf8').trim();
    if (signature !== sigFile) throw new Error(`${target} signature differs from ${asset}.sig.`);
    verifyArtifact({ bytes: readFileSync(join(assetsDir, asset)), fileName: asset, signature, publicKey, version });
  }
}

/** macOS only: the updater archive must hold exactly the arm64 app for `version` that trusts `publicKey`. */
export function inspectBundle({ archive, version, publicKey, appName = 'Rhythm.app', identifier = 'io.github.hiyabye.rhythm' }) {
  const work = mkdtempSync(join(tmpdir(), 'rhythm-updater-bundle-'));
  try {
    execFileSync('tar', ['-xzf', archive, '-C', work]);
    const entries = readdirSync(work);
    if (entries.join() !== appName) throw new Error(`Updater archive must contain only ${appName}; found ${entries.join(', ')}.`);
    const app = join(work, appName);
    const plist = (key) => execFileSync('plutil', ['-extract', key, 'raw', join(app, 'Contents/Info.plist')], { encoding: 'utf8' }).trim();
    if (plist('CFBundleIdentifier') !== identifier) throw new Error('Updater bundle has the wrong bundle identifier.');
    if (plist('CFBundleShortVersionString') !== version) throw new Error(`Updater bundle is version ${plist('CFBundleShortVersionString')}, expected ${version}.`);
    for (const binary of ['MacOS/rhythm', 'Resources/llama/metal/llama-server']) {
      const archs = execFileSync('lipo', ['-archs', join(app, 'Contents', binary)], { encoding: 'utf8' }).trim();
      if (archs !== 'arm64') throw new Error(`${binary} is built for ${archs}, expected arm64 only.`);
    }
    // The key is compiled in through option_env!; without it the shipped app cannot verify updates.
    if (!readFileSync(join(app, 'Contents/MacOS/rhythm')).includes(Buffer.from(publicKey.trim()))) {
      throw new Error('The application does not embed the configured updater public key.');
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

function main(argv) {
  const [command, ...rest] = argv;
  const options = {};
  for (let i = 0; i < rest.length; i += 2) {
    if (!rest[i]?.startsWith('--') || rest[i + 1] === undefined) throw new Error(`Invalid argument: ${rest[i]}`);
    options[rest[i].slice(2)] = rest[i + 1];
  }
  // Keys come from the environment so they never appear in process listings or logs.
  const publicKey = process.env.RHYTHM_UPDATER_PUBKEY;
  if (!publicKey?.trim()) throw new Error('RHYTHM_UPDATER_PUBKEY is not set.');
  if (command === 'artifact') {
    const signature = readFileSync(`${options.artifact}.sig`, 'utf8');
    verifyArtifact({ bytes: readFileSync(options.artifact), fileName: basename(options.artifact), signature, publicKey, version: options.version });
    console.log(`Verified ${basename(options.artifact)} for ${options.version}`);
  } else if (command === 'manifest') {
    const manifest = JSON.parse(readFileSync(options.manifest, 'utf8'));
    verifyManifest({ manifest, assetsDir: options.assets, publicKey, version: options.version, repo: options.repo, tag: options.tag });
    console.log(`Verified ${options.manifest} for ${options.tag}`);
  } else if (command === 'bundle') {
    // Overrides exist only for isolated validation builds; releases use the defaults.
    inspectBundle({ archive: options.archive, version: options.version, publicKey, appName: options['app-name'], identifier: options.identifier });
    console.log(`Inspected ${basename(options.archive)} for ${options.version}`);
  } else {
    throw new Error('Usage: verify-update.mjs bundle --archive <file> --version <v> | artifact --artifact <file> --version <v> | manifest --manifest <file> --assets <dir> --version <v> --repo <owner/name> --tag <tag>');
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(`Update verification failed: ${error.message}`);
    process.exit(1);
  }
}
