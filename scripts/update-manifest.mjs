// Builds the stable updater manifest (`latest.json`) from the staged release
// assets. verify-update.mjs checks the result independently before release.
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

export function buildManifest({ assetsDir, version, repo, tag, now = new Date() }) {
  const entry = (asset) => ({
    url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(asset)}`,
    signature: readFileSync(join(assetsDir, `${asset}.sig`), 'utf8').trim(),
  });
  const mac = entry('Rhythm.app.tar.gz');
  const windows = entry(`Rhythm_${version}_x64-setup.exe`);
  // No notes: Settings shows its own Korean text when the body is absent.
  return {
    version,
    pub_date: now.toISOString(),
    platforms: { 'darwin-aarch64': mac, 'darwin-aarch64-app': mac, 'windows-x86_64': windows, 'windows-x86_64-nsis': windows },
  };
}

const usage = 'Usage: update-manifest.mjs --assets <dir> --version <v> --repo <owner/name> --tag <tag> --output <file>';
const required = ['assets', 'version', 'repo', 'tag', 'output'];

export function parseArguments(rest) {
  const options = {};
  for (let i = 0; i < rest.length; i += 2) {
    if (!rest[i]?.startsWith('--') || rest[i + 1] === undefined) throw new Error(usage);
    options[rest[i].slice(2)] = rest[i + 1];
  }
  const missing = required.filter((name) => !options[name]);
  if (missing.length) throw new Error(`Missing ${missing.map((name) => `--${name}`).join(', ')}.\n${usage}`);
  return options;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  let options;
  try {
    options = parseArguments(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
  const manifest = buildManifest({ assetsDir: options.assets, version: options.version, repo: options.repo, tag: options.tag });
  writeFileSync(options.output, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`Wrote ${options.output} for ${Object.keys(manifest.platforms).join(', ')}`);
}
