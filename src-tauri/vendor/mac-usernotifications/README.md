# mac-usernotifications 0.3.1 — local response cleanup patch

This is the fixed 0.3.1 dependency used by Rhythm, including both notice and
Studium notifications. It retains the upstream `MIT OR Apache-2.0` license.
The original copyright and license texts are in `LICENSE-MIT` and
`LICENSE-Apache`; the upstream README is preserved as `README.upstream.md`.

## Provenance

- Upstream: <https://github.com/hoodie/mac-usernotifications>
- Published package: <https://crates.io/crates/mac-usernotifications/0.3.1>
- Package VCS revision: `5f63a95e317262761242efdde8abe1c1c4b0e819`
  (preserved in `.cargo_vcs_info.json`).
- Original crates.io package SHA-256 from Rhythm's lockfile before the local
  patch: `08adbb1b9e11709f9ce628dd7471208e5dd3ad35a245dbc81e59a36dce3f3780`.
- `src/` was copied from that registry package. License files were missing from
  its published include list and were obtained unchanged from the same VCS
  revision: [MIT](https://github.com/hoodie/mac-usernotifications/blob/5f63a95e317262761242efdde8abe1c1c4b0e819/LICENSE-MIT),
  [Apache-2.0](https://github.com/hoodie/mac-usernotifications/blob/5f63a95e317262761242efdde8abe1c1c4b0e819/LICENSE-Apache).

## Local changes

`NotificationHandle::response` now borrows the receiver through its owned
`PendingGuard`. The guard therefore lives for the complete response future and
its existing `Drop` unregisters the delegate sender on completion, timeout or
cancellation. The former `ManuallyDrop` receiver extraction skipped that cleanup
and also leaked the guard's request-id string. Public API, version, authorization
and native sender behavior are unchanged.

Cancellation also closes the receiver before unregistering it. Registration
checks for a closed receiver under the same registry lock, so an OS worker
queued before cancellation cannot recreate an abandoned registration afterward.

Three synthetic unit tests register only Rust oneshot channels and verify cleanup
after dropping a polled or unpolled response future and after late registration
following cancellation. They do not initialize the native notification worker,
request permission, display an alert or invoke an OS
notification callback. They require a macOS target to compile and run because
the library itself links Apple frameworks. A Windows cross-check cannot run
them or establish native notification behavior.

The original authored manifest is `Cargo.toml.upstream`. The local normalized
manifest omits examples, their bundle metadata and unused development
dependencies; examples are not vendored. Publishing is disabled. Production
dependencies, features and source files otherwise remain at the package pin.

On macOS, run the dependency tests with:

```sh
cargo test --locked -p mac-usernotifications --lib
```

Re-review this patch and its cleanup tests before replacing the pinned package.
