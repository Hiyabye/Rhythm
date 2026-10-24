# Rhythm development

## Scope and architecture

Rhythm is a Korean-first, local-first Tauri 2 desktop command center. Read `docs/ARCHITECTURE.md` before changing component boundaries and `docs/ROADMAP.md` before adding product scope. Current implementation is v0.2.0: a native-Rust, manually synchronized Sogang notice archive. No notice AI processing, model installer, scheduler, notifications, external web crawler or cloud service is implemented.

- `src/`: Svelte 5 runes, TypeScript and Tailwind/custom CSS. `App.svelte` owns navigation/settings; `lib/ArchiveView.svelte`, `DocumentReader.svelte` and `SourceSettings.svelte` own archive surfaces. `lib/types.ts` mirrors command payloads.
- `src-tauri/src/main.rs`: Tauri command registration, application state/startup/exit; `core/mod.rs` owns the data-directory lock and platform menu.
- `src-tauri/src/archive/`: shared record models, catalog, independent native parsers, constrained HTTPS/TLS, manual job controller and archive commands. `fixtures/README.md` and `certificates/README.md` document source/certificate provenance.
- `src-tauri/src/storage/`: rusqlite settings and archive APIs, FTS/query and upgrade tests; ordered migrations are in `src-tauri/migrations/`.
- `src-tauri/src/extensions/`: built-in metadata, registry and lifecycle; opt-in is separate from subscriptions/support.
- `src-tauri/src/intelligence/`: Tokio AI process ownership and protocol client. Archive collection does not invoke it.
- `crates/ai-worker/`: isolated mistral.rs runtime and JSON-lines executable. Read its `README.md` before worker/inference changes.
- `scripts/desktop.mjs`: host worker build/staging and Tauri invocation.
- `assets/branding/beat.svg`: vector icon source; committed desktop icons are in `src-tauri/icons/`.

There are only two Cargo workspace members. Do not add a crate or generic extension/plugin infrastructure without a concrete need. Source-specific parsing stays outside generic storage and reader code.

## Setup and commands

Use stable Rust **1.90+**, Node.js **22.12+**, npm and the OS-specific [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). Keep `cargo` and `rustc` on PATH. Use committed root `Cargo.lock` and `package-lock.json`.

```sh
npm ci
npm run desktop                      # CPU worker staging + native development app
npm run desktop:build                # release worker + native application/bundles
npm run desktop:build -- --debug      # debug worker/application/bundle
npm run desktop:build -- --no-bundle  # native build without installer generation
npm run check                        # Svelte/TypeScript diagnostics
npm run build                        # frontend production build only
cargo fmt --all
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

Tauri checks need `src-tauri/binaries/rhythm-ai-worker-<host-triple>[.exe]`. The desktop scripts create it; CI stages a debug worker before Clippy/tests. Direct Tauri invocation on a fresh checkout bypasses this step. Generated sidecars are ignored; icons are committed. The helper targets the host and assumes Cargo's default workspace `target/` directory.

For core real-worker lifecycle/crash-isolation tests, build the worker and set `RHYTHM_WORKER_TEST_BIN` to its absolute executable path before `cargo test -p rhythm`. Without it, the core real-worker test returns early; a passing suite alone does not establish process integration. Worker process tests launch their own test-built executable. Actual-model inference is separately ignored and opt-in via `RHYTHM_TEST_MODEL`; never claim it verified without real output.

Default worker builds include mistral.rs 0.8.1 CPU inference. Metal/CUDA are optional; initial Metal compilation was blocked by missing Apple Metal Toolchain. Do not install toolchains or download weights implicitly. See `crates/ai-worker/README.md` for local inference and known limits.

### Archive verification

Ordinary tests are network-free. Useful focused commands:

```sh
cargo test --locked -p rhythm storage::archive_tests
cargo test --locked -p rhythm archive::parser
cargo test --locked -p rhythm archive::sync
cargo test --locked -p rhythm archive::tls
```

`src-tauri/src/archive/sync/tests.rs::bounded_live_import_and_offline_reopen` is ignored deliberately. When bounded public-site verification is within the approved task, run it explicitly:

```sh
cargo test --locked -p rhythm bounded_live_import_and_offline_reopen -- --ignored --nocapture
```

It uses a fresh temporary directory by default. Optional `RHYTHM_LIVE_ARCHIVE_DIR` must name a fresh archive location; never point it at the user's app data. It imports at most one article and one listing page per supported source, leaves history incomplete, then reopens the DB without the client and checks local text/search/user state and resource hashes. Record actual output; changing live content means prior counts are not a permanent expectation. This test is not a full-history import or a native-reader test. The separately ignored `live_main_api_uses_verified_completed_chain` performs a bounded TLS/API retrieval only.

Do not run unbounded live collection as part of ordinary tests/CI. Prefer deterministic official-page fixtures and a fake transport at the network boundary. Relevant regressions include schema-1 upgrade/rollback, source-ID isolation, Korean literal queries, meaningful revisions, repeated pins, more-than-three-page new-record bursts, cancellation/disable before commit, pending-resource resume, offline reopen and integrity/repair. Only a real native interaction can establish reader/browser/file-reveal behavior.

## Data and lifecycle invariants

- Persist through Rust/SQLite, not frontend localStorage. Browser previews must state desktop-only limitations instead of simulating success.
- Database is `app_data_dir()/rhythm.sqlite3`; resources are generated local paths below `app_data_dir()/resources/`. Never hardcode home paths or use remote filenames as filesystem paths.
- Acquire the process-lifetime OS data-directory lock **before** opening/recovering the store. Controller admission additionally allows one archive/retry job. Do not open a second live production store to inspect progress: `Store::open` recovers persisted `running` sources as interrupted.
- Schema 2 follows released `001_foundation.sql` via `002_archive.sql`. Never rewrite released migrations. Require ordered transactional migration/upgrade/rollback tests and reject unknown newer schemas without modifying them.
- Document identity is `(source_id, external_id)`. Preserve original fetched representations, sanitized/normalized content, meaningful revisions and independent read/saved state. Fetch/parse/resource failures must not replace good content or erase user state.
- Catalog has 32 sources; only main `sogang-academic` and five `computing-*` sources are supported. Working list extraction alone is not support. Require verified full article/resource extraction and fixtures before changing support flags.
- Historical checkpoints advance only after article retrievals on a page succeed; resume pending resources for existing text. Mark history complete only at the last public page, never at a test/safety limit. Resource completeness is separate.
- Incremental discovery must continue past the three-page edit window while regular records are new; repeated pins cannot establish the known frontier. First incremental seed is historical, not a flood of new notices. It is not a full audit of old edits.
- `sogang_notices` is operational but opt-in and source subscriptions are separate. Restoring/enabling preferences starts no network work. `stop` signals cancellation; the owning async disable/exit path waits without holding the SQLite lock. Hide source controls while disabled, reject new downloads, retain archives/subscriptions and allow local read/search/save.
- Never retain SQLite mutex guards across async network/worker/filesystem operations. Check cancellation/extension enablement at commit boundaries.

## Security and implementation constraints

- Keep user-facing UI, feedback and errors in natural Korean. Internal IDs/docs may use English. Keep `src/lib/types.ts` and Rust payloads synchronized; validate backend inputs.
- Automatic fetches use exact approved HTTPS hosts/routes, public DNS with pinned answers, validated redirects, pacing/robots checks, cancellation, size/deadline/retry limits. Preserving HTTP(S) resource metadata is not permission to fetch it. No authentication/access-control bypass, unrestricted remote image fetches or downloaded-code execution.
- `archive/tls.rs` supplements only the exact `www.sogang.ac.kr` intermediate chain. Intermediates are **untrusted candidates**, not roots. Preserve Mozilla roots and normal WebPKI hostname/validity/signature/usage/path checks. Read `src-tauri/src/archive/certificates/README.md` before certificate changes; never repair connectivity by accepting invalid certificates or promoting intermediates to roots.
- Render only sanitized reader content; original responses are archive data. No script/style/iframe/event-handler content or remote-image fallback. Local images require bounded raster bytes and SHA-256 verification. External HTTP(S) links require explicit user action through the OS browser. File reveal must not execute attachments.
- Retain MIME/size/hash/state metadata, generated paths and last-good verified resource references. Retry must repair corrupt bytes rather than assume a hash filename proves integrity. Report failed/oversized/unsupported resources without discarding useful text.
- No localhost AI HTTP, cloud inference/storage, telemetry or separate inference service. Model ownership stays in the worker; stdout is exclusively versioned protocol and diagnostics go to stderr. Preserve CPU startup without weights/GPU tooling; do not equate one tested artifact with model/backend compatibility.
- Reference collector licensing is unestablished. Do not copy its implementation under an assumed MIT license. Official notice/fixture content retains its original rights.
- Use Rustfmt; frontend code must pass `npm run check` without warnings. No separate frontend formatter/linter is configured.

## Verification and documentation

Run affected checks, inspect the final diff and report exact evidence/limits. At minimum, settings changes need persistence/reopen tests; worker changes need process protocol/lifecycle tests; archive changes need the affected migration/parser/sync/offline regressions. Foundation CI success does not establish v0.2 CI. Native compilation does not establish rendering, accessibility, offline reader behavior, full historical coverage or Windows/Linux runtime parity. An ignored test or a real-worker test that returned early is not verification.

Keep `docs/ARCHITECTURE.md`, `docs/DESIGN.md` and `docs/ROADMAP.md` accurate when claims change. README is for users and basic run/build instructions; development workflows belong here. Mark release gates complete only after the corresponding checks actually ran. Do not add community-management templates, changelogs, publishing/updater infrastructure or future-release behavior outside scope.
