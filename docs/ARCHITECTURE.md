# Architecture

## Boundaries

Rhythm is a Korean-first, local-first modular monolith: one Tauri main window and, while a model is loaded, one supervised `llama-server` child process. It is not a web service. There is no cloud backend, login service or telemetry, and the inference server is reachable only by Rhythm (a private Unix socket, or loopback with a per-launch key).

Supported platforms are macOS Apple Silicon and Windows 11 x64. Platform code sits behind `cfg(target_os = "macos")`/`cfg(windows)`; both platforms share the bundle identifier `io.github.hiyabye.rhythm`, the schema and the backup format. The app data root (`<data>` below) is `app_data_dir()` on macOS and `app_local_data_dir()` (non-roaming `%LOCALAPPDATA%`) on Windows.

```text
Svelte 5 / TypeScript
        │ Tauri invoke
Rust application (src-tauri)
  ├─ storage: SQLite settings, archive, revisions, user state, FTS5, AI, personalization, proactive state
  ├─ extensions: built-in registry and lifecycle
  ├─ archive: source catalog, native parsers, constrained HTTPS, sync controller
  │     ├─ public Sogang sources → normalized archive records
  │     └─ <data>/resources/ → verified local files
  ├─ proactive: in-process scheduler and notifications
  └─ intelligence: curated model manager, analysis queue, llama-server supervisor
        ├─ <data>/models/ or a chosen folder → verified managed GGUF files
        ├─ stored document revision → validated result → SQLite cache
        │ HTTP/1.1 over <data>/run/llama.sock, or 127.0.0.1 + per-launch key
one llama-server process per loaded model (bundled resources, pinned upstream build)
  ├─ macOS: llama/metal/ → Metal, or CPU after a failure
  └─ Windows: llama/vulkan/ (NVIDIA only) or llama/cpu/
```

The Cargo workspace has one member, the Tauri app (`src-tauri`). Collection never waits for inference or writes AI results. Release builds are ad-hoc signed (macOS) or unsigned (Windows) by policy; updates are authenticated separately by the Tauri updater signature, and the updater stays disabled in builds without a compiled-in public key.

## Application ownership and commands

`src/App.svelte` owns navigation, the six settings panes (one rendered at a time; the last pane is kept while the app runs), nonblocking first-run AI setup and themes. In `src/lib/`: `ArchiveView.svelte` owns lists, search, Today recommendations and recent-analysis previews; `DocumentReader.svelte` reads a stored revision and takes document feedback; `SourceSettings.svelte` manages subscriptions and jobs; `PersonalizationSettings.svelte` owns profile, interests, learning and memory; `AiSettings.svelte` manages models, preferences and queue; revision-keyed `AnalysisPanel.svelte` shows results independently of the reader; `ProactiveSettings.svelte` exposes lifecycle and scheduler controls (`part="background"`) or notification controls and history (`part="notifications"`); `Popover.svelte` anchors a native light-dismiss popover to its `popovertarget` button and `Hint.svelte` uses it for row explanations. `types.ts` mirrors Rust command payloads. `platform.ts` picks platform wording from the WebView user agent (never for security decisions).

Commands:

- Foundation: `get_snapshot`, `save_settings`, `set_extension`, `worker_status`.
- Archive (`archive/mod.rs`): `archive_sources`, `archive_subscribe`, `archive_summary`; `archive_query`, `archive_document`, `archive_state`; `archive_start`, `archive_cancel`, `archive_status`; `archive_retry_resource`, `archive_resource_image`, `archive_reveal_resource`; `archive_open_original` (explicit HTTP(S) navigation in the OS browser only).
- Proactive: `proactive_snapshot`, `proactive_save`, `proactive_history`, `proactive_clear_history`, lifecycle status and explicit close-to-background/hidden-start/login updates, `notification_authorization` (status, no prompt) and `notification_request_permission` (explicit user action only).
- AI (`intelligence/commands.rs`): `ai_snapshot`; install, cancel, register, remove, select, load, unload and restart; `ai_preferences`; `ai_analyze`, `ai_cancel`; revision-local results and jobs; bounded recent results; `ai_cancel_operation` for command-owned preparation; `ai_diagnostics` for transient timing reports; `ai_choose_model_directory` and `ai_reset_model_directory`.
- Personalization: `personalization_get`, `_save_profile`, `_save_interest`, `_remove_interest`, `_set_learning`, `_reset`, `_get_feedback`, `_set_feedback`, `_recommendations`.

`AppState` owns an `Arc<Mutex<Store>>`, the archive controller and client, the extension registry, one shared Tokio-mutex worker, model-install and analysis controllers, a transient operation owner and an AI mutation gate. SQLite guards are never held across network, worker or large-file hashing awaits; hashing uses `spawn_blocking`. Health snapshots use `try_lock` instead of waiting behind inference. Search and reader generation guards keep stale responses from replacing newer views.

`intelligence/operations.rs` owns in-memory load/setup/register phases and cancellation independently of Settings' lifetime, so leaving and returning to Settings shows the same operation. Cancellation joins cooperative blocking verification or stops and reaps an interrupted server before admission is released. Setup persists completion only after its real inference succeeds. No database job kind is added for these operations.

Before `Store::open` can recover interrupted work, `core::lock_data_dir` takes a process-lifetime OS file lock on `<data>/rhythm.lock`. A second owner gets a Korean diagnostic and exits normally before opening the store or worker. Single-instance handling runs before that lock. The OS releases the lock when the process ends; an old lock file alone means nothing.

Rust owns persistence; browser previews never substitute localStorage or simulate success. A startup or migration failure stops launch instead of replacing user data. Theme changes are optimistic and revert on save failure.

## Storage and schemas

`storage/mod.rs` opens `<data>/rhythm.sqlite3`, enables foreign keys, WAL and a five-second busy timeout, and applies ordered transactional migrations by `PRAGMA user_version`. A newer schema is rejected before the journal mode changes. Released migrations are never rewritten.

| Migration | Adds |
| --- | --- |
| `001_foundation` | Settings and extension preferences |
| `002_archive` | `sources`, `documents`, `document_revisions`, `document_state`, `resources`, `documents_fts` (FTS5 trigram over current title and body) |
| `003_intelligence` | `ai_models`, `ai_preferences`, `ai_jobs`, `ai_results` |
| `004_personalization` | Profile, interests, feedback, evidence, activity, suppression, `source_extensions`; `documents.last_revision_at` |
| `005_proactive` | Lifecycle/scheduler/notification settings, per-source tasks, revision ceiling, dedupe ledger, bounded event history, `sources.seed_complete` |
| `006_notification_attempts` | Bounded attempt ledger and nondecreasing UTC watermark |
| `007_notification_activation` | Hashed, expiring Windows activation tokens |
| `008_ai_execution_mode` | `ai_preferences.execution_mode` (`auto`/`cpu`/`gpu`, used only on Windows) |
| `009_ai_model_directory` | `ai_preferences.model_directory`, `ai_models.storage_directory` |
| `010_update_check` | `settings.update_check` (default on), `settings.update_checked_at` (last successful automatic check, UTC) |
| `012_resource_retry` | Data only: `too_large` rows (mostly mislabeled by an exhausted job budget) and images blocked only by the former route list become `pending` |
| `011_skipped_articles` | `skipped_articles`: notices whose fetched response was rejected for its content, with listing URL, original historical marking, reason and response SHA-256 |

### Archive tables

| Table | Purpose |
| --- | --- |
| `sources` | Catalog identity, subscription, support flag, last attempt/success/error, per-attempt counters, resumable page, history and seed completion |
| `documents` | Current identity and metadata, sanitized HTML and normalized text, unique on `(source_id, external_id)` |
| `document_revisions` | Append-only meaningful snapshots, including the original fetched JSON/HTML, reader content and hash |
| `document_state` | Read/saved flags and timestamps, independent of content |
| `resources` | Per-revision image/attachment metadata, download state, local path, size, MIME, SHA-256 |

`Store::upsert_article` commits document, revision, user-state initialization and resource references in one transaction. The revision hash covers canonical URL, title, author, category, publication date, sanitized body/text and resource identities, not view counts, collection time or raw response churn. An unchanged check only updates validators and check time. Revisions never replace read/saved state, and old snapshots stay selectable. A downloaded resource is reused only for the same document, URL and role, so a remote file overwritten at the same URL is not detected; explicit retry can replace that revision's reference. There are no deletion/tombstone semantics or automatic retention cleanup.

Original responses are stored as revision data and never injected into the webview. A parse, network or write failure never replaces the last good article.

On reopen, running source jobs become `interrupted` and keep their page; running AI jobs become retryable failures; interrupted downloads become cancelled but keep partial bytes and validators. Pending AI jobs stay persisted and drain on later analysis admission, not at startup.

Settings shows database plus WAL size; archive summaries separately total referenced resource bytes, deduplicated by path. Neither is a full disk-usage report.

### AI tables

| Table | Purpose |
| --- | --- |
| `ai_models` | Managed/external ownership, file identity, pinned provenance, download state, resume ETag, errors, storage directory |
| `ai_preferences` | Selected model, setup completion, explicit/recent policy, automatic session limit, output token limit, execution mode, model directory |
| `ai_jobs` | Attempts with priority, status, error, duration and full semantic identity; a partial unique index prevents duplicate active work |
| `ai_results` | Validated JSON, provenance, revision and cache identity, duration, partial-processing counts; inserted in the same transaction as job completion |

Jobs and results use the composite `(revision_id, document_id)` foreign key. Results keep artifact provenance without depending on inventory, so deleting a model keeps its analyses. A failed or cancelled regeneration keeps earlier good output. Revision-local job queries return all active attempts plus a bounded terminal history (20 in the reader); bounded history is never used to decide deletion safety.

## Personalization and ranking

`004_personalization.sql` keeps the legacy `settings.interests` text and imports its terms into structured rows; equivalent Korean/English labels share one normalized identity with aliases. The legacy value is cleared only by an explicit personalization reset. `documents.last_revision_at` changes only when a meaningful revision is committed.

| Table | Purpose |
| --- | --- |
| `personalization_profile` | Optional university, department, study level; learning switch |
| `personalization_interests` | Explicit or learned topic, normalized identity, aliases, priority 1–5, polarity, evidence summary |
| `personalization_feedback` | One idempotent row per document (`interested` / `not_interested`) |
| `personalization_evidence` | Deduplicated save/positive-feedback events per topic, document and kind |
| `personalization_activity` | Monotonic interaction sequence for refreshing learned memory |
| `personalization_suppressed` | Sequence baseline so removed inferred interests do not return from unchanged evidence |
| `source_extensions` | Source → built-in extension; a missing or disabled extension removes the source from recommendations, not from the archive |

Only saves and explicit positive feedback are behavioral evidence; opening, reading, searching and negative feedback create no topic memory. Learning needs evidence on at least two different documents, expires evidence after 180 days and recomputes after changed feedback/save state or at the next snapshot after a day. Negative feedback adjusts only its document and never hides a source or topic. Explicit interests are never overwritten by learned ones, and canonical explicit aliases prevent duplicate learned rows. Editing a learned memory promotes it to explicit; removing one records a sequence baseline.

Evidence topics come only from titles and original source categories, at most 32 terms per action, filtered by a small Korean/English sensitive-term list (lexical, not semantic). Body text and AI output are never used.

`Store::personalization_recommendations` is deterministic and never reads AI results or calls the worker:

- Each matching explicit topic: `12 + 3×priority` (15–27). Learned topic: at most 6. Explicit negative topics subtract their amount.
- Positive feedback: +24 for 30 days, +12 to day 180, +6 to day 365. Negative feedback caps the item score at −40 to day 180, −20 to day 365 (the item is not hidden).
- Saved +8; subscribed source +2; exact university/department/study-level match +4/+8/+2.
- Freshness: non-historical publication 0–6 over 30 days; recently collected older or undated non-historical +2; meaningful revision of an older item in the last three days +4. Historical backfill gets no freshness.
- AI dates, categories and summaries are never scored.

Candidates come from bounded indexed slices: 500 latest published, 250 recent non-historical, 250 meaningful updates, 100 saved, 100 feedback, 250 source, 250 category and 250 FTS matches (at most 1,950 before deduplication). At most 32 terms are matched against FTS5; each candidate exposes only a 320-character body prefix. Today requests eight rows; scores are not persisted. Explanations come from the same scoring branches. Order is score, then publication time, then document ID, descending. A first pass limits one source to half the list; a second pass fills the rest in score order. Personal evidence and results never leave the device or enter diagnostics.

## Korean local search

`storage/archive.rs::archive_query` searches title and normalized body, never navigation or footer text. Whitespace-separated terms are ANDed; each may match title or body. Terms of three or more characters provide quoted trigram MATCH candidates; parameterized, escaped LIKE checks enforce substring semantics for every term and handle one- and two-character Korean terms. Punctuation is data, not query syntax. Embedded NUL is rejected. Limits: 512 query characters, 32 terms, 100 rows per page (the UI uses 30).

Results have deterministic publication-date/ID order, source/category and saved/unread filters, and a plain-text excerpt. `Highlighted.svelte` renders text and `mark` nodes, not HTML from SQLite. Search and user state work offline and with the notice extension disabled. The ignored 5,000-record benchmark checks correctness and reports warm debug timings (about 11 ms inbox, 17 ms short Korean, 52 ms broad mixed queries on the development Mac); it is not a speed gate.

## Sources and parsers

`archive/catalog.rs` has 32 stable source identities. **Catalog presence is not support.** Ten have verified list and detail extraction, local resources and offline reopen:

| Source ID | Board |
| --- | --- |
| `sogang-academic` | 서강대학교 학부 학사공지 (main API board 2) |
| `computing-academic` | 소프트웨어융합대학 학사 공지 |
| `computing-graduate` | 소프트웨어융합대학 대학원 공지 |
| `computing-external` | 소프트웨어융합대학 대외정보 |
| `computing-updates` | 소프트웨어융합대학 소식 |
| `computing-career` | 소프트웨어융합대학 취업·인턴십 |
| `eng-general` | 공과대학 일반공지 (`/board/notice`) |
| `eng-careers` | 공과대학 취업정보 (`/board/recruit`) |
| `eng-academic` | 공과대학 학사공지 (`/board/academic`) |
| `eng-research` | 공과대학 연구공지 (`/board/research`) |

The other 22 (Engineering newsletter, Electronic, Mechanical, System Semiconductor and Semiconductor boards) are shown as unsupported and cannot be subscribed. Support needs verified article and resource extraction plus fixtures, not just a working list URL.

`parser.rs` validates board/article identity, metadata, dates, pagination and response structure. The main board uses public paginated JSON list/detail endpoints; HTML error pages are not accepted as JSON, and listings must agree on size, total, page, pins and duplicates. Computing uses desktop `.board-list table` rows (so mobile pin copies don't distort pagination), extracts `article.board-detail`, and reads attachment metadata from a restricted subset of serialized Nuxt data without running JavaScript; its public route ID, internal `DETAIL.SN` and file path must all agree, and inline image roles win when an attachment shares the URL. `engineering.rs` validates the current SSR canonical/form/board/pagination identity, ordinals, full title and body, attachment names and visible Korean time against machine UTC. Unexpected structure is an explicit source failure, never an empty success. Fixture provenance is in `archive/fixtures/README.md`.

Two reference collectors by the owner, [SogangFriends](https://github.com/Hiyabye/SogangFriends) and [sogang-notices](https://github.com/Hiyabye/sogang-notices), informed the catalog and parser contracts. Neither has an explicit code license, so Rhythm's Rust is written independently; observed public contracts are not permission to copy their code. Official notice and fixture content keeps its original rights.

A new notice whose body is only iframes (blank text and textless `p`/`div`/`span`/`br` wrappers allowed, as in the school's `div.iframe-wrapper` PDF viewers) is archived as metadata plus the original representation, with empty sanitized fields and an explicit unsupported-body state. The iframe is never fetched or rendered, and existing readable content is never replaced by such a result: that notice is recorded as checked and is neither a failure nor a skip. Readable content means non-empty sanitized HTML or text, or an image reference of the current revision whatever its download state; a stored image reference is kept because a later retry or route change can still fetch it.

## Synchronization, scheduler and lifecycle

`archive/sync.rs::Controller` admits one archive or resource-retry job at a time. Admission checks the enabled extension and supported subscribed sources under the same store lock that disabling uses. Sources run sequentially; one source's failure is recorded and does not stop the next.

- **Historical:** start at the persisted `next_page`, follow public pagination, deduplicate pins and IDs, and checkpoint only after no article on the page failed transiently (see below). Existing text is not refetched, but pending resources resume. `history_complete` becomes true only at the last public page, never at a safety or test limit.
- **Incremental:** start at page one and recheck recent pages and pins with ETag/Last-Modified validators. The minimum recheck window is three pages; discovery continues past it until a page of known regular records marks the frontier. Repeated pins never establish the frontier.
- **Refresh:** the manual action (새로고침) runs one job per board: an incremental pass first so new notices appear quickly, then a historical pass only while `history_complete` is false. Either step's failure or cancellation stops that board and keeps what the earlier step saved. The scheduler never uses it, so automatic work cannot start a backlog download.
- **First incremental seed:** with no local documents, fetch at most three pages as historical records, without counting them as new or marking history complete.
- **Scope:** incremental sync is not an audit of every old edit.

**Article failures.** A *transient* failure (network, deadline, server error, cancellation, redirect, or a response of the wrong content type, which is more likely a block page than the notice) holds the page checkpoint, fails the board and is retried on the next run. A *permanent* failure belongs to the fetched response: the body does not parse or its encoding is unsupported. It is recorded in `skipped_articles` (reason and response SHA-256), any last-good content stays, and the checkpoint advances, so one bad notice cannot stop a board. A later fetch that archives or confirms the notice (including HTTP 304) removes the row. Skipped notices are listed per board (count plus the 20 most recent) and are never dropped silently. Notices on recent pages are refetched by every incremental run; others only by the explicit `skipped` mode, which requests just those detail pages, archives them with their recorded historical marking, and leaves the board's status, counters, checkpoint and last success unchanged.

Counters describe the current attempt; checkpoints are cumulative. HTTP 304 updates the check time and still resumes pending resources. A resource failure is nonfatal to article archival but surfaces as a failure on explicit retry. Completing pagination does not promise every resource was downloaded.

The scheduler (`proactive/scheduler.rs`) is an in-process Tokio task that wakes every 15 seconds, not an OS service. When enabled and unpaused it reconciles tasks for supported, subscribed sources while the extension is enabled, and runs due sources sequentially with incremental sync. Interval `0` follows the energy profile (Eco/Balanced/Performance: 60/30/15 minutes); explicit intervals are exact values from 15 to 720 minutes. Retry interval is 5–120 minutes (default 15). Outcomes, next-run times and errors persist. Disabling, pausing, changing subscriptions or the extension, and shutdown all cancel work. Nothing runs while the computer sleeps or the app is closed. Scheduled results are tied to the admitted controller task, and the scheduler keeps servicing notification discovery while an import runs, without starting a second archive job.

`core/lifecycle.rs` owns tray, window and login behavior. Every lifecycle feature defaults off. Login registration changes only on explicit user action, is checked against actual OS state and rolls back if persistence fails. The registration is named after the bundle identifier: the production identifier keeps the entry released builds already created (macOS `Rhythm.plist`, Windows Run value `Rhythm.io.github.hiyabye.rhythm`), and every other identifier gets its own entry, so a validation or test build cannot read, replace or delete the installed app's login registration. A login launch honors hidden start; a manual launch stays visible unless `--hidden` is passed. Close-to-background works only if the tray installed; otherwise closing quits. Exit signals scheduler and archive cancellation, waits without holding SQLite, then stops the worker.

`sogang_notices` is opt-in and separate from subscriptions. Disabling hides source controls and rejects new work but keeps archives, subscriptions, reading, search and saving.

## Notifications

Notifications are opt-in and independent of the scheduler. Discovery runs after archive commits. Candidate identity is document + revision + kind. Migration 005 set a revision ceiling so upgrades never announce old notices; a historical first seed is excluded, but a later meaningful revision of an old notice can qualify. Policy, source and negative-interest checks repeat before delivery; rejections are stored as `suppressed` with a reason. Events reference archive identities and read titles from archived revisions; no notice body is copied.

| Setting | Values |
| --- | --- |
| Sensitivity | `important` (preferences and score ≥18), `interested` (explicit-interest match), `all` (any eligible item). Negative feedback or negative-interest matches always suppress |
| Quiet hours | Local wall-clock range (default off, 22:00–07:00); eligible events are deferred, not consumed |
| Rate | At most three attempts per ten minutes, bounded candidate batches |

Event states: `pending → claimed → attempted | deferred | failed`, then `delivered` after activation confirmation, or `suppressed`. Claims have a five-minute lease; delivery attempts cap at three with capped exponential backoff (maximum four hours). Schema 6 reserves rate slots in a durable ledger before handoff, so clearing or pruning history never resets rate accounting or dedupe. A crash between reservation and handoff can use a slot without a visible notification. A backward clock change pauses the watermark until wall time catches up.

- **macOS:** native UserNotifications through the async `mac-usernotifications` 0.3.1 sender. A handoff is only `attempted`; an event is `delivered` only after the activation response. Responses are processed independently of the scheduler's delivery cycle: each alert closes itself after 30 minutes and at most 12 responses are tracked, so an ignored alert cannot stall later work. Authorization status maps not-determined/denied/authorized/provisional/ephemeral (timeout or error = unavailable). Denied or undetermined permission blocks attempts, but quiet-hours deferral still applies.
- **Windows:** WinRT toasts with the installed AUMID and `rhythm-win://notification/<token>` protocol activation; the protocol registration must resolve to the expected executable first. Status is authorized/denied/unavailable from notifier settings and install identity; there is no prompt, and the explicit button sends one test notification. The final policy check, token creation, attempt reservation and synchronous handoff share one store lock. Schema 7 stores only a hash of the random, expiring, one-use token bound to the event attempts; a validated click for the current attempt marks delivery. Discovery claims nothing unless authorization is `authorized`. Clicks during shutdown are refused, and restore clears tokens in staging.
- **Other hosts:** no sender; status is unavailable.

A click restores the existing window and emits `proactive://notification-clicked` with stable archive identities. The main-window capability grants only event `listen`/`unlisten`; `App.svelte` unlistens on teardown and falls back to a one-shot snapshot activation if the click arrived before it subscribed. `ProactiveSnapshot.notification_permission` stays `"unknown"`; Settings uses the separate authorization commands. Notification text includes the source name and title. No AI request is involved.

## Transport, resources and safe rendering

`archive/network.rs` uses native Reqwest/Rustls. Automatic requests need exact approved HTTPS hosts and routes on port 443 (main, Computing and Engineering). URLs with credentials, fragments or unsafe encoded separators are rejected. DNS answers must be public and are pinned per connection; proxies and automatic redirects are off. The sync policy layer follows at most three same-origin redirects, re-running URL/DNS/TLS checks and robots authorization before each hop; transport only returns redirect targets. HTTP(S) resource references are kept as metadata so an old HTTP image doesn't discard the article, but only approved HTTPS routes are downloaded. Main images under `/dataview/board/<id>/<file>.(png|jpg|jpeg|gif|webp)` are an approved route. Requests carry an access mode: `Source` allows only those routes; `EmbeddedImage`, used for image resources only, also allows any other public HTTPS host name on port 443 (no IP literals, no other `*.sogang.ac.kr` route). Each such origin's robots.txt is loaded once per job and cached (including a failure); a 4xx robots.txt from such a host means no rules (RFC 9309), while a school host's error stays fatal. Third-party attachments are not downloaded.

Requests are paced at least 650 ms apart. `robots.txt` is checked before collection (plain-text Accept header; Computing returns 406 otherwise); allow/disallow rules with wildcard and end-anchor matching and crawl delays up to 60 seconds apply, and malformed delays or HTML error pages fail closed. Transient errors get at most three attempts with short backoff, connect/request/attempt deadlines and cancellable waits. HTTP 429 stops with a message. No login, access restriction or anti-bot bypass is used.

### TLS chain completion

`www.sogang.ac.kr` omits its intermediate certificate. `archive/tls.rs` adds two bundled public intermediates as **untrusted candidates, only for that exact host**, then delegates all checks to WebPKI. The root store stays exactly Mozilla's `webpki_roots`; hostname, validity, signature, usage and path checks stay mandatory. No other host gets candidates. There is no invalid-certificate acceptance, new root or system trust change. Provenance and fixed-time tests are in `archive/certificates/README.md`.

### Resources

Limits: 4 MiB per list/article response, 10 MiB per inline image, 20 MiB per attachment, 256 MiB of completed resource bytes per job (charged before validation or disk writes; when the remaining budget, not the per-file cap, bounds a download, a size failure leaves the file `pending` for a later run), 64 KiB for robots, and a 100,000-page safety ceiling. Test budgets report `limited`, never complete history.

Resource URL, original filename, kind, MIME, bytes, state and SHA-256 are kept. Files use generated `resources/<sha256>.<validated-extension>` paths, never remote filenames, and are written through a `.part` file before SQLite references them. Images need PNG/JPEG/GIF/WebP signatures, PDFs a PDF signature; attachment extensions are restricted and active web types rejected. This is format validation, not antivirus scanning. Files are never opened or executed automatically; reveal only selects them in Finder or Explorer.

The `resources` sync mode ("자료 모두 내려받기") downloads `pending`/`failed` files of each subscribed board's current revisions, newest notice first, until the job budget is spent; like `skipped`, it leaves board status, counters, checkpoint and last success unchanged.

Local reads check containment, bound size and verify the stored SHA-256. Retry repairs corrupt bytes; a failed refresh keeps the last verified copy. There is no remote-image fallback. Cleanup of unreferenced files and storage quotas are not implemented.

The Rust sanitizer keeps a limited structural HTML subset, strips scripts, iframes, forms, styles and event attributes, and emits image references without `src`. Recursion stops at depth 64: a deeper subtree (legacy editors nest tables about 130 levels) is rendered iteratively as one block of its text and images without structure. More than 50,000 nodes is a parse error. `DocumentReader.svelte` builds a second restricted DOM, requests only integrity-checked local raster data URIs, and sends HTTP(S) links to the OS browser on explicit action. Missing or corrupt images become placeholders with repair actions. A failed automatic read-state write never hides content. CSP is an extra layer, not a replacement for the sanitizer.

## Managed models and downloads

`intelligence/models.rs` owns curated downloads, separate from notice transport. `catalog.json` pins Qwen3.5 2B/4B Q4_K_M revisions, files, sizes and SHA-256 (see [AI evaluation](AI_EVALUATION.md#pinned-artifacts)). One-click installation accepts only `verified` entries. Inventory rows don't depend on the catalog: a row whose catalog entry was removed (9B, 0.8B) still opens, selects, removes and restores, and the UI marks it as no longer offered; its bytes are never deleted implicitly.

Downloads use HTTPS to exact approved Hugging Face/CDN hosts with pinned public DNS per hop, normal TLS, no proxy and at most five reviewed redirects. A disk check reserves 1 GiB beyond the remaining bytes. Paths are generated: `models/<sha256>.part` and `models/<sha256>.gguf` under the default folder or the model's recorded custom destination (`model_storage.rs`). Resume needs a strong ETag, `Range`/`If-Range`, an exact `Content-Range` and the expected remaining size; a full `200` restarts. Files sync and pass GGUF preflight, exact size and committed SHA-256 before an atomic rename.

`ai_choose_model_directory` opens the native folder picker, validates the folder and write access, then saves it; cancelling changes nothing. On Windows any link or junction in the path is rejected; on macOS only a linked selected or `models` folder (system links such as `/var` are canonicalized). A new download captures its destination; existing and interrupted downloads keep theirs. A pre-existing final file in a chosen folder is adopted only if that inventory already received every byte, so removal never deletes bytes Rhythm didn't write. Both folder commands share AI/maintenance admission; `ai_preferences` cannot change the folder.

External registration references a canonical `.gguf` path without copying. Preflight accepts GGUF v2/v3 `qwen3`/`qwen35` metadata; it is not full tensor or runtime validation. Registration computes the full SHA-256, and re-registering a changed file unloads cached weights first. Unregister never deletes external bytes. Managed removal checks containment, removes partial files first, records `missing` before the irreversible unlink and reconciles failures; documents and results stay. Loading and uncached analysis verify the complete file every time; the `sha2` assembly feature (macOS aarch64 only) speeds hashing but never replaces it.

## Inference process

`intelligence/server.rs` supervises upstream `llama-server`, bundled as Tauri resources under `llama/` and pinned to one llama.cpp build with per-archive size and SHA-256 (`scripts/llama-cpp.json`). `Worker` in `intelligence/mod.rs` owns it behind the same status/load/infer/unload/stop API the rest of the app uses. No process runs until a model is loaded or a request needs one; status without a process only checks that the bundled executable exists.

A load starts `llama-server -m <model> --no-webui --no-slots --jinja -np 1 -c <context> -lv 4` with all layers on the GPU, or `-ngl 0 --device none` for CPU. On Windows the Vulkan build first lists devices and runs only on an NVIDIA one. The server listens on `<data>/run/llama.sock` inside a `0700` directory. When that path exceeds the 103-byte Unix socket limit, or on Windows, it listens on 127.0.0.1 with an OS-chosen port read from its log, and a random key in a `0600` file (deleted once the server is ready) is required on every request. The supervisor waits up to 180 s for `GET /health`, then reads the device and offloaded layer count from the log to report the loaded backend. A Vulkan load counts as GPU only when every layer was offloaded.

Requests are one HTTP/1.1 exchange per connection to `/v1/chat/completions`, with a 600 s deadline: a fixed system instruction, the prompt, greedy sampling, thinking disabled and, for analysis, the closed generation schema as a JSON-schema `response_format`. Responses need an exact `Content-Length` and are bounded to 4 MiB. Token counts and durations come from the server's `timings`. A context-overflow error maps to a Korean message containing `토큰 한도를 넘습니다`. Stdout is discarded; stderr is drained as bounded diagnostic lines.

One model and one sequence at most. Loading another model, unloading and cancelling kill and reap the process; a late response is never reused, and the next request starts a fresh one. A crash is observed on the next status or request and never blocks archive or settings access. On Windows the process is in a kill-on-close job object.

macOS tries Metal first and falls back to CPU for the session after a load or request failure. On Windows the execution mode picks the build: `gpu` reports errors, `cpu` uses the CPU build, and `auto` falls back to CPU for the session after a device, load or request failure, but retries only that request on CPU after a context overflow (Vulkan context 8192 tokens; CPU and Metal 32768).

## Analysis, validation and cache

`intelligence/analysis.rs` reads a stored revision through `Store::document`; it never refetches the page or opens attachments. One persistent queue serializes inference. The default is explicit requests; opening an article only reads existing results. Cache lookup happens before any model inspection or server launch, and pending duplicates share one attempt.

**Cache identity:** document/revision + task + artifact SHA-256 + prompt version + output-schema version + a hash of output-affecting configuration (token limit, source bound, selector and evidence encoding, prompt rules, clock rules, decoding mode, thinking, sampling, repair policy, the `llama-server-v1` engine marker; plus the execution mode on Windows). The engine marker makes results from the former worker read as outdated. Current prompt version is **3**, selector **head-tail-keywords-v4**, stored schema **1**. Older results stay readable with provenance but never satisfy a newer key; the reader warns when a stored result's contract differs from the current one. Recovered pending attempts with an obsolete identity fail with retry guidance.

**Output (stored schema 1):** strict JSON with `short_summary`, `summary`, one of eight `category` values, qualitative `uncertainty` and `facts`. Fact kinds are deadline, event date, eligibility, documents, application method, link and action. Prompt 3 sends selected source passages as `{id,text}` objects, and generation must return exactly `kind`, `label`, `value` and integer `evidence_id` per fact. The backend resolves each ID to the exact archived text and derives date and uncertainty before validating the stored schema. Invalid references, unknown or missing fields, invalid categories, malformed, empty, oversized or repetitive output, control tokens, unsupported evidence and ungrounded links are rejected.

**Dates:** a deadline or event date is normalized only from a single explicit date that appears in both the value and its passage, matched on digit boundaries. Claimed clock times must be supported for that date, with Korean and English AM/PM handled (`meridiem-v2`). Missing years, relative dates, ranges and multiple dates stay null. Literal grounding is not semantic entailment, and uncertainty is not calibrated confidence.

**Repair:** one schema or grounding failure allows one bounded repair with validation feedback. A second failure fails the job without replacing earlier good results; runtime errors never loop. Both generations are cancellable.

**Input bounds:** title and body ranges are selected from head, tail and important passages within 18 KiB of encoded source inside a 24 KiB prompt. Over budget, exact repeated passages are admitted once; table rows are separate evidence units; ordinary passages break at sentences or lines (500-character hard bound). Output is 128–4096 tokens (default 1024). Long documents record `partial` with processed and total characters, and the reader warns that omitted parts may hold dates or requirements. No chunk-and-merge, attachment text or vision.

The model gets a fixed system instruction and JSON-quoted untrusted source data, no tools. Generated text renders only as escaped Svelte text. There are no model-driven commands or external actions.

Diagnostics are transient: at most 20 terminal reports in memory with stage timings, inference metrics and the fixed validator message of each failed validation, no source or generated text. Validator messages are written for the repair prompt; a job that still fails validation after the repair stores one fixed user-facing error instead.

## Energy policy

`intelligence/maintenance.rs` checks every five seconds while the app runs. Optional `recent` processing needs completed setup, an installed selected model and a focused main window. It considers at most 20 eligible latest revisions by collection time after session start, excluding historical backfill, startup history and revisions with any prior attempt. Attempts are capped per session.

| Profile | Automatic analysis | Idle model residency | Scheduler cadence when interval is `0` |
| --- | --- | --- | --- |
| Eco | None (explicit only) | 30 s | 60 min |
| Balanced | Up to `min(automatic_limit, 3)` | 120 s | 30 min |
| Performance | Up to `min(automatic_limit, 20)` | 600 s | 15 min |

Active work, a busy operation gate or model/job changes reset residency. Battery state is not detected; Eco disables automatic AI on AC and battery.

## Backup, restore and updater

`.rhythm-backup` is a ZIP with `manifest.json` (version 1, located by name, written last) listing each payload path, size and SHA-256. Creation copies the database with SQLite's online backup API in one step while holding the store's only connection, and adds only database-referenced resources; `models/` is excluded and external models stay references. Each file is read once and every resource must match its recorded size and SHA-256. An interrupted backup's snapshot is removed at the next launch. The archive is not encrypted.

Restore needs an awaited native confirmation (`window.confirm` is an async shim in the desktop app and must not be used). It extracts into hidden staging, verifies SQLite integrity, foreign keys and resource hashes, reconciles models (a managed model stays only if its local file matches the restored size and SHA-256; custom-folder models become external or missing references, never deleted; the folder preference is cleared), clears Windows activation tokens, and writes a journal. Rhythm then exits instead of relaunching, because a relaunched instance can race the exiting process for the single-instance socket and data lock. The next manual launch runs `recover_pending_restore` before `Store::open`, finalizes idempotently and shows the outcome. A recovery error shows a native dialog and exits, keeping the journal and previous data. The previous profile is kept in `.rhythm-restore-previous-*` under the data root. `stage_restore` is the whole pre-exit path and is what the ignored large-profile benchmark measures. Each extracted file is flushed with `sync_all` (`F_FULLFSYNC` on macOS); Unix also syncs directories after each rename.

On Windows, restore additionally requires a local fixed NTFS volume, rejects reparse points, junctions and Cloud Files sync roots, moves with `MoveFileExW` (no replace or copy fallback) and stops on ambiguous recovery artifacts. Without directory fsync it survives process interruption, not power loss.

Backup, restore and updater share an app-wide `AdmissionGate`: maintenance closes admission, cancels and joins scheduler, archive and AI work, then reopens.

The updater verifies Tauri-signed artifacts against a build-time public key and requires the signed version to match the manifest (`requireSignedVersion`). The plugin registers only when `RHYTHM_UPDATER_PUBKEY` is set at build time; otherwise Settings shows manual update steps (`updater_configured`). Errors are mapped to Korean (`update_error`). The single endpoint is the `latest.json` of the latest published GitHub release; its `darwin-aarch64[-app]` entries point at the signed `Rhythm.app.tar.gz` and its `windows-x86_64[-nsis]` entries at the signed NSIS setup executable. The updater signature is not OS code signing: Gatekeeper and SmartScreen still treat the builds as unsigned on first install. `updater_restart` holds maintenance admission and stops and reaps a loaded `llama-server` before `install`, because the server runs from the install folder and the Windows plugin exits with `process::exit` (no `RunEvent::Exit`). Windows runs the NSIS installer in passive mode (`/P /UPDATE /R`). Install-location failures (read-only volume, translocated app, denied or cancelled authorization, an old bundle that could not be put back) map to Korean guidance.

Automatic checks (`run_auto_check`, keyed builds only, on by default) wake one minute after launch and then hourly, and check when the setting is on and the last successful check is a day old. They use a 30-second timeout, fail silently, keep the found update in `updater::State` and emit `updater://available`; the main window shows a dismissible banner that opens Settings → 데이터 및 정보. They never download or install.

## Packaging and CI

`src-tauri/tauri.conf.json` bundles `src-tauri/binaries/llama/` as the `llama/` resource directory. `scripts/stage-llama.mjs` fills it: it downloads the pinned archives for the host from llama.cpp's GitHub releases into `target/llama-cache/`, rejects any whose size or SHA-256 differs from the pin, and copies `llama-server` with only the libraries it loads (macOS) or with its DLLs plus the VC runtime it imports (Windows). `scripts/desktop.mjs` stages, then runs Tauri; it is not a cross-compilation helper. On Windows it delegates to `scripts/desktop-windows.mjs` (VC runtime, llama.cpp, NSIS); see [Windows](WINDOWS.md). Scripts never install toolchains or weights.

A default release build deletes `target/release/bundle/` and writes `target/release/distribution/`: the DMG or setup executable and its `.sha256`, plus the updater package files when both `TAURI_SIGNING_PRIVATE_KEY` and `RHYTHM_UPDATER_PUBKEY` are set (`scripts/updater-signing.mjs`). Release builds remap `$HOME`, `CARGO_HOME` and the checkout with `--remap-path-prefix`. `scripts/macos-release.mjs` (Developer ID signing and notarization) is kept for a possible future decision and refuses to run without credentials; `scripts/update-manifest.mjs` builds `latest.json` from the staged signatures and `scripts/verify-update.mjs` independently verifies signatures, the manifest, the macOS archive and the embedded public key.

`ci.yml` runs each concern once, gated by changed paths (`scripts/ci-changes.mjs`): frontend checks and Playwright on Ubuntu, `cargo fmt` on Ubuntu, and Clippy plus tests on macOS 14 and Windows. Release builds run only on `main` or manual runs when packaging inputs change. `release.yml` builds both platforms on a version tag with the updater key from the reviewer-gated `release` environment, verifies the updater files and `latest.json`, and creates a draft release; publishing it makes the update visible to installed apps. Normal tests are network-free; opt-in live, benchmark and model tests are listed in `AGENTS.md`.
