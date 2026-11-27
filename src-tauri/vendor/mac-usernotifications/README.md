# mac-usernotifications 0.3.1 (local response cleanup patch)

Rhythm uses this fixed 0.3.1 copy for notice and Studium notifications. It keeps the upstream `MIT OR Apache-2.0` license. The license texts are in `LICENSE-MIT` and `LICENSE-Apache`. The upstream README is kept as `README.upstream.md`.

## Provenance

- Upstream: <https://github.com/hoodie/mac-usernotifications>
- Published package: <https://crates.io/crates/mac-usernotifications/0.3.1>
- Package VCS revision: `5f63a95e317262761242efdde8abe1c1c4b0e819` (kept in `.cargo_vcs_info.json`).
- Original crates.io package SHA-256, from Rhythm's lockfile before the patch: `08adbb1b9e11709f9ce628dd7471208e5dd3ad35a245dbc81e59a36dce3f3780`.
- `src/` was copied from that registry package.
- The published package left out the license files. They come unchanged from the same revision: [MIT](https://github.com/hoodie/mac-usernotifications/blob/5f63a95e317262761242efdde8abe1c1c4b0e819/LICENSE-MIT), [Apache-2.0](https://github.com/hoodie/mac-usernotifications/blob/5f63a95e317262761242efdde8abe1c1c4b0e819/LICENSE-Apache).

## Local changes

**Response cleanup.** `NotificationHandle::response` now borrows the receiver through its owned `PendingGuard`. The guard lives for the whole response future. Its existing `Drop` unregisters the delegate sender on completion, timeout or cancellation.

The old `ManuallyDrop` receiver extraction skipped that cleanup and leaked the guard's request-id string.

Public API, version, authorization and native sender behavior are unchanged.

**Cancellation.** Cancellation closes the receiver before unregistering it. Registration checks for a closed receiver under the same registry lock. An OS worker queued before cancellation therefore cannot recreate an abandoned registration.

**Tests.** Three synthetic unit tests register only Rust oneshot channels. They check cleanup after dropping a polled or an unpolled response future, and after a late registration that follows cancellation.

The tests do not start the native notification worker, ask for permission, show an alert or call an OS notification callback. They need a macOS target, because the library links Apple frameworks. A Windows cross-check cannot run them and does not show native notification behavior.

**Manifest.** The original manifest is `Cargo.toml.upstream`. The local manifest is normalized. It leaves out examples, their bundle metadata and unused dev-dependencies, and the examples are not vendored. Publishing is disabled. Production dependencies, features and source files otherwise stay at the package pin.

## Testing

On macOS:

```sh
cargo test --locked -p mac-usernotifications --lib
```

Re-review this patch and its cleanup tests before replacing the pinned package.
