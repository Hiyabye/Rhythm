// Release builds produce signed updater artifacts only when both halves of
// the updater key are present. One without the other is a misconfigured
// release: the app would trust a key nothing is signed with, or the reverse.
// Returns the extra Tauri CLI arguments, or null for an ordinary build.
// `bundle` adds platform settings, such as the targets the updater needs.
export function updaterSigningArgs(env, bundle = {}) {
  const privateKey = env.TAURI_SIGNING_PRIVATE_KEY?.trim();
  const pubkey = env.RHYTHM_UPDATER_PUBKEY?.trim();
  if (!privateKey && !pubkey) return null;
  if (!privateKey || !pubkey) {
    throw new Error('Updater signing needs both TAURI_SIGNING_PRIVATE_KEY and RHYTHM_UPDATER_PUBKEY.');
  }
  // The committed config has no key; the app compiles the same key in
  // through option_env!, and the CLI checks its signature against this one.
  return ['--config', JSON.stringify({ bundle: { ...bundle, createUpdaterArtifacts: true }, plugins: { updater: { pubkey } } })];
}
