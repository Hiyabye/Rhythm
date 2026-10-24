# Architecture

## Boundaries

Rhythm is a modular monolith with a single Tauri main window and one isolated AI process. It is not a web service. There is no cloud backend, login service, telemetry client, or localhost inference server.

```text
Svelte 5 / TypeScript
        │ Tauri invoke
Rust application (src-tauri)
  ├─ storage: SQLite settings and extension preferences
  ├─ extensions: built-in registry and lifecycle
  └─ intelligence: worker process owner
        │ versioned JSON lines over stdin/stdout
rhythm-ai-worker (crates/ai-worker)
        └─ mistral.rs 0.8.1 → an explicitly supplied local GGUF
```

The root Cargo workspace contains only the Tauri application and worker. This separation isolates inference crashes and model memory without creating a service or many internal crates.

## Frontend and application core

`src/App.svelte` owns navigation, the settings view, transient search input, and theme application. `src/lib/types.ts` describes the frontend IPC data. The frontend calls four commands in `src-tauri/src/main.rs`:

- `get_snapshot`: settings, registered extensions, storage information, and last known worker status.
- `save_settings`: validated theme, energy profile, and explicit interest text.
- `set_extension`: registry-mediated preference/lifecycle transition.
- `worker_status`: actively asks the worker for its status.

Rust owns persistent state. Browser previews never substitute localStorage or pretend desktop persistence is available. Theme changes are optimistic and revert on save failure; OS theme changes are observed when system mode is selected. The energy profile is stored and returned to callers but does not change scheduling in this release.

`AppState` holds the SQLite store behind a standard mutex and the worker behind a Tokio mutex. No mutex guard over SQLite is retained across an asynchronous worker call. Errors at storage boundaries are logged locally and surfaced as Korean messages. A database initialization failure prevents startup rather than silently replacing the user's database.

## SQLite ownership and migrations

`src-tauri/src/storage/mod.rs` opens `rhythm.sqlite3` inside Tauri's `app_data_dir()` for `io.github.hiyabye.rhythm`. Settings exposes the resolved path; application code does not hardcode home directories. SQLite is bundled through rusqlite, with FTS5 available. No document index exists yet.

Migration `src-tauri/migrations/001_foundation.sql` creates only:

- `settings`: singleton theme, energy profile, and interest text; enum values have SQL constraints.
- `extension_preferences`: stable extension identifier and enabled preference.

The store enables foreign keys, WAL journaling, and a five-second busy timeout. `PRAGMA user_version` tracks the schema. The first migration runs transactionally; reopening the current schema does not recreate tables. A schema newer than the application is rejected to prevent an older binary from altering it. Future migrations must be ordered, transactional, and tested against existing databases; never edit a released migration to implement an update.

Storage size currently reports the database plus its WAL, not a future attachment/model directory or total application disk usage. SQLite WAL requires coordinated snapshots or the SQLite backup API for future backup support: copying only the live `.sqlite3` file is not a reliable backup design.

### Evolution, not premature schema

v0.2.0 should add sources, documents, revisions, normalized/original content, attachment references, read/saved state, and synchronization metadata as ingestion actually needs them. FTS5 can index normalized archive content. Later migrations can add generated annotations, user feedback, relevance scores, and persistent jobs. None of these tables is implied to exist today. Model weights should remain separate from both the installer and SQLite rows.

## Built-in extensions

`src-tauri/src/extensions/mod.rs` defines an `Extension` lifecycle (`metadata`, `start`, `stop`) and a registry. Metadata includes stable identifier, Korean name/description, availability, and the user's persisted enabled preference. Settings renders the registry rather than a separate hardcoded extension list.

`sogang_notices` is registered with `available: false`. Its preference can be changed, but no network connector is started. Availability is distinct from opt-in. The core is university-independent; a future connector must provide normalized records through core-owned interfaces rather than modifying unrelated UI/storage code.

The lifecycle contract requires `stop` to cancel jobs and notifications before returning. Disabling does not delete preference rows or downloaded content. There are no jobs, notifications, extension-specific views, or document data to cancel/hide today. Implementations added later must enforce those semantics and restore enabled/available extensions at startup. There is no dynamic loader, marketplace, or external plugin ABI.

## Worker protocol and lifecycle

The main process locates `rhythm-ai-worker` beside its own executable (`.exe` on Windows), launches it inside the Tokio runtime, pipes stdin/stdout, and drains a separate stderr pipe for diagnostics. Windows launch suppresses a separate console window. The child is kill-on-drop. Initial status is requested during setup. Worker failure changes AI status without removing ordinary settings/data access. Snapshots inspect child exit status; protocol checks detect EOF and malformed responses. While Settings is open, the frontend checks worker health every five seconds, so status does not stay falsely ready after a crash. There is no autonomous restart supervisor.

Protocol version 1 uses one UTF-8 JSON object per line:

```json
{"version":1,"id":1,"method":"status"}
{"version":1,"id":1,"ok":true,"result":{"engine":"mistral.rs","runtime_available":true,"model":null}}
```

Methods are `ping`, `status`, `infer`, and `shutdown`. Errors use `ok: false` and a Korean `error` string. Requests are serial and matched by numeric ID/version. The worker caps request frames at 1 MiB and validates local model paths, prompt length, and output token limits. Full details and an actual-inference test are in [`crates/ai-worker/README.md`](../crates/ai-worker/README.md).

The app currently sends lifecycle/status commands only; inference is an explicit worker test, not a UI feature. Lifecycle requests have eight-second deadlines. A failed refresh kills and reaps the worker. Normal application exit stops extensions, requests worker shutdown, waits briefly, and forcibly terminates a child that fails to exit. No tray process is intentionally retained. In-flight inference cancellation and streaming are deferred.

mistral.rs belongs only to the worker. Its default feature includes the real CPU runtime; a lifecycle-only `--no-default-features` build accurately reports no runtime. At most one model is retained; changing the path releases the previous model before loading another. Existing local GGUF directories are used with `TokenSource::None`, not remote model identifiers. No weight or tokenizer download is initiated by Rhythm. Upstream contains unconditional stdout prints, so the worker preserves a dedicated protocol descriptor and redirects process stdout to stderr before initializing inference.

## Portability and packaging

`src-tauri/tauri.conf.json` declares the target-specific worker as a bundled `externalBin`. `scripts/desktop.mjs` builds it, derives the host target using `rustc -vV`, stages the suffixed sidecar, then invokes Tauri. Use the desktop scripts rather than invoking Tauri directly on a fresh checkout. This helper currently builds the host target, not arbitrary cross-compilation targets.

Native controls and webviews come from Tauri. macOS has a Korean application/edit/window menu. Windows and Linux use normal native window controls without unsupported macOS hide/menu actions. Paths and process execution use Rust/Node platform APIs. CPU is the portable inference baseline; `metal` and `cuda` are optional worker features. Windows/Linux/GPU runtime parity is not assumed. Metal compilation on the initial Mac was blocked by a missing Apple Metal Toolchain; no automatic toolchain installation was attempted. Real GGUF CPU generation and loaded-model shutdown were verified with Qwen3-0.6B-Q8_0.gguf on macOS Apple Silicon. Qwen3.5 compatibility remains unverified.

CI declares frontend checks and Rust checks/builds on macOS, Windows, and Linux. A green configuration file is not a green run; installers, signatures, updater infrastructure, and release publication remain separate validation work. Nothing in this design requires shipping model weights inside application updates.
