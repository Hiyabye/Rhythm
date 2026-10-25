# Rhythm development

## Scope and architecture

Rhythm is a Korean-first, local-first Tauri 2 desktop command center. Read `docs/ARCHITECTURE.md` before changing component boundaries and `docs/ROADMAP.md` before adding product scope. Current implementation is v0.3.0 Local Intelligence: the hardened manual Sogang archive plus model management, persistent local analysis and app-open energy controls. Model/native release verification is still in progress; implementation is not proof of supported Qwen3.5 artifacts. Collector provenance is in `docs/COLLECTOR_COMPARISON.md`; exact model/evaluation status is in `docs/AI_EVALUATION.md`. No persistent background scheduler, notifications, external web crawler or cloud service is implemented.

- `src/`: Svelte 5 runes, TypeScript and Tailwind/custom CSS. `App.svelte` owns navigation/settings; `lib/ArchiveView.svelte`, `DocumentReader.svelte` and `SourceSettings.svelte` own archive surfaces; `AiSettings.svelte` and revision-keyed `AnalysisPanel.svelte` own AI management/results. `lib/types.ts` mirrors command payloads.
- `src-tauri/src/main.rs`: Tauri command registration, application state/startup/exit; `core/mod.rs` owns the data-directory lock and platform menu.
- `src-tauri/src/archive/`: shared record models, catalog, independent native parsers, constrained HTTPS/TLS, manual job controller and archive commands. `fixtures/README.md` and `certificates/README.md` document source/certificate provenance.
- `src-tauri/src/storage/`: rusqlite settings/archive/AI inventory/job/result APIs, FTS and upgrade tests; ordered migrations are in `src-tauri/migrations/`.
- `src-tauri/src/extensions/`: built-in metadata, registry and lifecycle; opt-in is separate from subscriptions/support.
- `src-tauri/src/intelligence/`: worker client, `models.rs`/pinned `catalog.json`, single `analysis.rs` queue, Tauri `commands.rs` and app-open `maintenance.rs`. Synthetic evaluation fixtures/tests are `evaluation.json`/`evaluation.rs`. Collection never waits for AI.
- `crates/ai-worker/`: isolated mistral.rs runtime and JSON-lines executable. Read its `README.md` before worker/inference changes.
- `scripts/desktop.mjs`: host worker build/staging and Tauri invocation.
- `assets/branding/beat.svg`: vector icon source; committed desktop icons are in `src-tauri/icons/`.

There are only two Cargo workspace members. Do not add a crate or generic extension/plugin infrastructure without a concrete need. Source-specific parsing stays outside generic storage and reader code.

## Setup and commands

Use stable Rust **1.90+**, Node.js **22.12+**, npm and the OS-specific [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). Keep `cargo` and `rustc` on PATH. Use committed root `Cargo.lock` and `package-lock.json`.

```sh
npm ci
npm run desktop                      # Metal if Apple Silicon tooling is available, otherwise CPU
npm run desktop:build                # release worker + native application/bundles
npm run desktop:build -- --debug      # debug worker/application/bundle
npm run desktop:build -- --no-bundle  # native build without installer generation
npm run check                        # Svelte/TypeScript diagnostics
npm run build                        # frontend production build only
npm run test:ui                      # compiled frontend + mocked IPC, Chromium/WebKit
npm run test:desktop                 # local command stubs: backend selection and staging (POSIX)
cargo fmt --all
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

Tauri checks need `src-tauri/binaries/rhythm-ai-worker-<host-triple>[.exe]`. The desktop scripts create it; CI stages a debug worker before Clippy/tests. Direct Tauri invocation on a fresh checkout bypasses this step. Generated sidecars are ignored; icons are committed. The helper targets the host and assumes Cargo's default workspace `target/` directory.

UI tests use the dev dependency `@playwright/test` and require its Chromium/WebKit binaries (`npx playwright install chromium webkit`; CI also installs OS prerequisites). `playwright.config.ts` builds/serves the actual frontend at 127.0.0.1:4178; tests mock only the Tauri boundary and must not be reported as native backend verification. Test output/traces stay under ignored `node_modules/.cache/`. Do not rely on a running preview server; the suite owns its server. Maintain meaningful keyboard/focus, failure-state, paging and reader regressions alongside Rust/native checks.

For core real-worker lifecycle/crash-isolation tests, build the worker and set `RHYTHM_WORKER_TEST_BIN` to its absolute executable path before `cargo test -p rhythm`. Without it, the core real-worker test returns early; a passing suite alone does not establish process integration. Worker process tests launch their own test-built executable. Actual-model inference is separately ignored and opt-in via `RHYTHM_TEST_MODEL`; never claim it verified without real output.

Direct Cargo worker builds default to CPU inference with mistral.rs v0.9.1 pinned to release commit `11f255058573e085afe0a149f8a19cbd39aa1200`; this inspected release adds the required `qwen35` GGUF route. Desktop scripts prefer Metal on Apple Silicon only when the existing compiler probe succeeds; otherwise CPU. `RHYTHM_AI_BACKEND=cpu|metal|cuda` overrides selection. Apple Metal Toolchain was installed with explicit approval. Pinned Qwen3.5 2B/4B Q4_K_M produced useful structured Korean output on Metal; this does not verify CPU/CUDA, 9B or other platforms. Do not install toolchains or download weights implicitly. See `crates/ai-worker/README.md` and `docs/AI_EVALUATION.md`.

### Intelligence verification and invariants

Focused deterministic checks:

```sh
cargo test --locked -p rhythm storage::intelligence_tests
cargo test --locked -p rhythm intelligence::analysis
cargo test --locked -p rhythm intelligence::models
cargo test --locked -p rhythm intelligence::maintenance
cargo test --locked -p rhythm-ai-worker
```

Normal tests must not download weights. The explicitly ignored `real_curated_model_install` exercises the production downloader with a pinned experimental entry only for validation. With approved weight/disk budget, set `RHYTHM_MODEL_INSTALL_ID` and `RHYTHM_MODEL_INSTALL_DIR` to a dedicated test directory. Never target production app data. Use `RHYTHM_MODEL_EVAL_ROOT`/`RHYTHM_MODEL_EVAL_ID`, `RHYTHM_WORKER_TEST_BIN` and `RHYTHM_AI_EVAL_REPORT` for ignored `actual_qwen35_analysis_evaluation`; details and bounds are in `docs/AI_EVALUATION.md`. Build the specified worker executable explicitly before running process tests; a prior test or feature build may leave a different backend/lifecycle binary at the same path.

- Catalog entries are pinned immutable repository revisions/files/size/SHA-256, not arbitrary URLs. Do not promote `experimental` to `verified` without real artifact/backend inference and useful-output evidence. 2B/4B are promoted with an explicit macOS Apple Silicon/Metal qualification; 9B remains experimental and installation-gated.
- Model transport is separate from archive transport. Keep HTTPS exact-host/path checks, public DNS/pinned answers, reviewed CDN redirects, disabled proxies and committed digest validation. Safe resume requires strong ETag and exact range validation; use generated `.part` paths and atomic finalization. Disk admission reserves 1 GiB beyond remaining weights.
- One model/sequence and one analysis at a time. Never cancel serial IPC by dropping its future and reusing the pipe: persist cancellation, stop/reap, then restart on demand. Unload reaps/restarts to reclaim process memory. Exit must also interrupt command-owned load/setup inference.
- Model registration computes identity, but preflight only accepts GGUF v2/v3 `qwen3`/`qwen35`; it is not complete tensor/runtime validation. Re-registering a changed external file must unload cached weights before output can claim its new hash.
- Cache identity includes document/revision, task, artifact hash, prompt/schema versions and semantic configuration hash. Preserve previous good results on failed regeneration and after model deletion; never use a bounded terminal-history window as the authoritative active-job/deletion check.
- Prompt version 2 generates passage `evidence_id` references, resolved to exact source bytes before validating stored schema 1. Derive normalized dates deterministically from unique explicit dates in value and evidence. Allow at most one bounded validation-repair attempt. Validate strict structured JSON and exact evidence against the selected processed revision. Missing/ambiguous dates remain unknown; no generated percentages, source-metadata replacement, tool calls or external actions. Render generated text only through escaped Svelte expressions.
- Long-input handling is a bounded prefix, not complete chunked analysis: record/display partial counts. Source bound is 18 KiB JSON-escaped text within a 24 KiB prompt. No binary-attachment extraction or remote refetch for analysis.
- Maintenance runs only while the app is open. Explicit-only is default; opt-in recent analysis requires focus/setup/model, excludes startup/history, examines a newest-20 publication window and uses a per-session cap. Eco starts no automatic work; idle model limits are 30/120/600 seconds for Eco/Balanced/Performance. Do not claim battery detection or full recent-ingestion coverage.

### Archive verification

Ordinary tests are network-free. Useful focused commands:

```sh
cargo test --locked -p rhythm storage::archive_tests
cargo test --locked -p rhythm archive::parser
cargo test --locked -p rhythm archive::sync
cargo test --locked -p rhythm archive::tls
cargo test --locked -p rhythm large_archive_operations -- --ignored --nocapture
```

The ignored benchmark ingests 5,000 generated records through actual Store APIs, verifies pagination/filter/index correctness and reports warm debug samples. It is not a live crawl, native IPC or a timing gate. Storage failure tests use an isolated SQLite page cap/abort trigger, never a full host disk or production DB.

`src-tauri/src/archive/sync/tests.rs::bounded_live_import_and_offline_reopen` is ignored deliberately. When bounded public-site verification is within the approved task, run it explicitly:

```sh
cargo test --locked -p rhythm bounded_live_import_and_offline_reopen -- --ignored --nocapture
```

It uses a fresh temporary directory by default. Optional `RHYTHM_LIVE_ARCHIVE_DIR` must name a fresh archive location; never point it at the user's app data. It imports at most one article and one listing page per supported source, leaves history incomplete, then reopens the DB without the client and checks local text/search/user state and resource hashes. Record actual output; changing live content means prior counts are not a permanent expectation. This test is not a full-history import or a native-reader test. The separately ignored `live_main_api_uses_verified_completed_chain` performs a bounded TLS/API retrieval only.

Do not run unbounded live collection as part of ordinary tests/CI. Prefer deterministic official-page fixtures and a fake transport at the network boundary. Relevant regressions include schema-1 upgrade/rollback, source-ID isolation, Korean literal queries, meaningful revisions, repeated pins, more-than-three-page new-record bursts, cancellation/disable before commit, pending-resource resume, offline reopen and integrity/repair. Only a real native interaction can establish reader/browser/file-reveal behavior.

## Data and lifecycle invariants

- Persist through Rust/SQLite, not frontend localStorage. Browser previews must state desktop-only limitations instead of simulating success.
- Database is `app_data_dir()/rhythm.sqlite3`; resources and managed model files use generated paths below `resources/` and `models/`. External model references stay at canonical original paths and must never be deleted by unregister. Never hardcode home paths or use remote filenames as filesystem paths.
- Acquire the process-lifetime OS data-directory lock **before** opening/recovering the store. Controller admission additionally allows one archive/retry job. Do not open a second live production store to inspect progress: `Store::open` recovers running source/AI jobs and interrupted model downloads.
- Schema 3 appends `003_intelligence.sql` after released `001_foundation.sql`/`002_archive.sql`; never rewrite released migrations. AI tables stay separate from authoritative documents/settings. Require populated-old-schema upgrade, transactional rollback and newer-schema rejection tests; do not change a newer DB even to enable WAL.
- Document identity is `(source_id, external_id)`. Preserve original fetched representations, sanitized/normalized content, meaningful revisions and independent read/saved state. Fetch/parse/resource failures must not replace good content or erase user state.
- Catalog has 32 sources; main `sogang-academic`, five `computing-*`, and `eng-general`/`eng-careers`/`eng-academic`/`eng-research` are supported. Engineering uses current `/board/{notice,recruit,academic,research}` SSR routes, not retired CMS routes; `parser.rs` dispatches to its source-specific `engineering.rs` submodule. Working list extraction alone is not support. Require verified full article/resource extraction and fixtures before changing support flags.
- Historical checkpoints advance only after article retrievals on a page succeed; resume pending resources for existing text, including HTTP 304 article checks. Counters describe the current attempt; checkpoint/history state is cumulative. Resource failure outcomes are nonfatal to article archival but must surface as failure for explicit retry. Mark history complete only at the last public page, never at a test/safety limit. Resource completeness is separate.
- Incremental discovery must continue past the three-page edit window while regular records are new; repeated pins cannot establish the known frontier. First incremental seed is historical, not a flood of new notices. It is not a full audit of old edits.
- `sogang_notices` is operational but opt-in and source subscriptions are separate. Restoring/enabling preferences starts no network work. `stop` signals cancellation; the owning async disable/exit path waits without holding the SQLite lock. Hide source controls while disabled, reject new downloads, retain archives/subscriptions and allow local read/search/save.
- Never retain SQLite mutex guards across async network/worker/filesystem operations. Check cancellation/extension enablement at commit boundaries.

## Security and implementation constraints

- Keep user-facing UI, feedback and errors in natural Korean. Internal IDs/docs may use English. Keep `src/lib/types.ts` and Rust payloads synchronized; validate backend inputs.
- Automatic fetches use exact approved HTTPS hosts/routes, public DNS with pinned answers, same-origin redirects authorized by robots before every hop, pacing, cancellation, size/deadline/retry limits. Transport returns redirect targets; only the sync policy layer may follow them. Completed resource response bytes consume the job budget even if format validation or filesystem persistence fails. Preserving HTTP(S) resource metadata is not permission to fetch it. No authentication/access-control bypass, unrestricted remote image fetches or downloaded-code execution.
- `archive/tls.rs` supplements only the exact `www.sogang.ac.kr` intermediate chain. Intermediates are **untrusted candidates**, not roots. Preserve Mozilla roots and normal WebPKI hostname/validity/signature/usage/path checks. Read `src-tauri/src/archive/certificates/README.md` before certificate changes; never repair connectivity by accepting invalid certificates or promoting intermediates to roots.
- Render only sanitized reader content; original responses are archive data. No script/style/iframe/event-handler content or remote-image fallback. Local images require bounded raster bytes and SHA-256 verification. External HTTP(S) links require explicit user action through the OS browser. File reveal must not execute attachments.
- Retain MIME/size/hash/state metadata, generated paths and last-good verified resource references. Retry must repair corrupt bytes rather than assume a hash filename proves integrity. Report failed/oversized/unsupported resources without discarding useful text.
- No localhost AI HTTP, cloud inference/storage, telemetry or separate inference service. Model ownership stays in the worker; stdout is exclusively versioned protocol and diagnostics go to stderr. Preserve CPU startup without weights/GPU tooling. Compiled backend, loaded backend, artifact integrity, actual generation and useful Korean analysis are distinct verification claims.
- Both reference collectors' source licensing is unestablished. Do not copy its implementation under an assumed MIT license. Official notice/fixture content retains its original rights.
- Use Rustfmt; frontend code must pass `npm run check` without warnings. No separate frontend formatter/linter is configured.

## Verification and documentation

Run affected checks, inspect the final diff and report exact evidence/limits. At minimum, settings changes need persistence/reopen tests; worker changes need process protocol/lifecycle tests; archive changes need affected migration/parser/sync/offline regressions; AI changes need semantic-cache, grounding, cancellation/recovery and model-inventory/download boundary regressions. Foundation CI success does not establish v0.2 CI. Native compilation does not establish rendering, accessibility, offline reader behavior, full historical coverage or Windows/Linux runtime parity. An ignored test or a real-worker test that returned early is not verification.

Keep `docs/ARCHITECTURE.md`, `docs/DESIGN.md` and `docs/ROADMAP.md` accurate when claims change. README is for users and basic run/build instructions; development workflows belong here. Mark release gates complete only after the corresponding checks actually ran. The observed three-OS run is v0.2.0 (`40110222263`), not v0.2.1; new UI CI configuration is not evidence of a passing run. Linux distribution remains blocked by transitive glib 0.18.5 / RUSTSEC-2024-0429; do not suppress the advisory or force incompatible GTK dependencies. Do not add community-management templates, changelogs, publishing/updater infrastructure or future-release behavior outside scope.
