import { createHash } from 'node:crypto';

// Appends --remap-path-prefix flags to the build's Rust flags. `remaps` is a
// list of [from, to] pairs; the last matching remap wins.
export function remapFlags(env, remaps) {
  // Cargo prefers CARGO_ENCODED_RUSTFLAGS over RUSTFLAGS; keep either.
  const existing = env.CARGO_ENCODED_RUSTFLAGS
    ?? (env.RUSTFLAGS ?? '').split(/\s+/).filter(Boolean).join('\x1f');
  const flags = [existing, ...remaps.map(([from, to]) => `--remap-path-prefix=${from}=${to}`)]
    .filter(Boolean).join('\x1f');
  return { ...env, CARGO_ENCODED_RUSTFLAGS: flags };
}

export const sha256Hex = (bytes) => createHash('sha256').update(bytes).digest('hex');

// The `shasum -a 256` line for a distribution file.
export function checksumLine(name, bytes) {
  const digest = sha256Hex(bytes);
  return { digest, text: `${digest}  ${name}\n` };
}
