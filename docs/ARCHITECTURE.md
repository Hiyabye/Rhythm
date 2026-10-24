# Architecture

## Boundaries

Rhythm is a Korean-first, local-first modular monolith with one Tauri main window and one isolated AI worker. It is not a web service: no cloud backend, login service, telemetry, or localhost inference server is required.

```text
Svelte 5 / TypeScript
        │ Tauri invoke
Rust application (src-tauri)
  ├─ storage: SQLite settings, archive, revisions, user state and FTS5
  ├─ extensions: built-in registry and lifecycle
  ├─ archive: source catalog, native parsers, HTTPS and manual sync controller
  │     ├─ public Sogang sources → normalized archive records
  │     └─ app_data_dir()/resources/ → verified local files
  └─ intelligence: worker process owner
        │ versioned JSON lines over stdin/stdout
rhythm-ai-worker (crates/ai-worker)
        └─ mistral.rs 0.8.1 → an explicitly supplied local GGUF
```

There are only two Cargo workspace members: the Tauri app and worker. v0.2.0 adds the archive inside the existing main process, not another service or a transplanted Node collector. Notice collection never calls the AI worker. Generic records and storage remain separate from source-specific route/HTML extraction.

## Application ownership and commands

`src/App.svelte` owns navigation, settings and theme application. `src/lib/ArchiveView.svelte` handles archive lists/search; `DocumentReader.svelte` reads a stored revision; `SourceSettings.svelte` manages subscriptions and manual jobs. `src/lib/types.ts` mirrors Rust command payloads.

Foundation commands remain `get_snapshot`, `save_settings`, `set_extension`, and `worker_status`. `src-tauri/src/archive/mod.rs` adds:

- `archive_sources`, `archive_subscribe`, `archive_summary`: catalog, subscriptions and actual local counts.
- `archive_query`, `archive_document`, `archive_state`: search/list pages, a current or prior revision, and independent read/saved state.
- `archive_start`, `archive_cancel`, `archive_status`: admit, cancel and observe a manual job.
- `archive_retry_resource`, `archive_resource_image`, `archive_reveal_resource`: explicit downloads and integrity-checked local access.
- `archive_open_original`: explicit HTTP(S) navigation through the OS browser; no file, script or custom-protocol URL execution.

`AppState` owns an `Arc<Mutex<Store>>`, the archive controller/client, the extension registry, and a Tokio-mutex-protected AI worker. SQLite guards are not held across network or worker awaits. Search runs through `spawn_blocking`; frontend generation checks prevent stale search/reader responses from replacing a newer request.

Before `Store::open` can recover interrupted work, `core::lock_data_dir` obtains a process-lifetime OS file lock on `app_data_dir()/rhythm.lock`. A second owner of that data directory is rejected. The OS releases the lock after process termination; startup does not infer ownership from an old file's mere existence. This complements the controller's in-process single-job admission and prevents concurrent instances from racing checkpoints or temporary resource files.

Rust owns persistence; browser previews do not substitute localStorage or simulate success. A startup or migration failure prevents launch rather than replacing user data. Theme changes are optimistic and revert on save failure; system mode follows OS preference. Energy profile and interests are retained settings, not a v0.2 scheduler or AI personalization policy.

## SQLite schema 2 and upgrade compatibility

`storage/mod.rs` opens `app_data_dir()/rhythm.sqlite3` for `io.github.hiyabye.rhythm`, enables foreign keys, WAL and a five-second busy timeout, and applies ordered transactional migrations using `PRAGMA user_version`. A newer schema is rejected before changing journal mode. Reopening schema 2 does not recreate tables. The released `001_foundation.sql` is unchanged.

`002_archive.sql` adds only implemented archive structures:

| Structure | Ownership and purpose |
| --- | --- |
| `sources` | Stable catalog identity, persisted subscription, support flag, last attempt/success/error, progress counters, resumable page and history completion. |
| `documents` | Current canonical identity/metadata and sanitized HTML/normalized text, keyed uniquely by `(source_id, external_id)`. |
| `document_revisions` | Append-only meaningful content snapshots, including original fetched JSON/HTML representation, reader content and hash. |
| `document_state` | Read/saved flags and timestamps, independent of fetched content. |
| `resources` | Revision-associated image/attachment metadata, download state, local path, size, MIME and SHA-256. |
| `documents_fts` and triggers | FTS5 trigram index over current title and normalized body text. |

The schema-1 upgrade keeps theme, energy, interests and extension preferences. Catalog refresh updates definitions without replacing subscriptions or checkpoints. Previously running source jobs become `interrupted` on reopen and retain their page. No automatic job is started at launch.

`Store::upsert_article` commits the current document, revision, user-state initialization and resource references transactionally. Identity includes the source, so equal article IDs on different boards do not collide. Hash input includes canonical URL, title, author, category, publication date, sanitized body/text and resource identities—not volatile view counts, collection time or raw response churn. An unchanged check updates validators/check time without another revision. Revisions never replace read/saved state. Old snapshots and their resource references remain selectable.

Original responses are stored as revision data, not injected into the webview. A parse, network or write failure does not replace the last good article with an error page. There are no deletion/tombstone semantics or automatic retention cleanup in v0.2.0.

Database size in Settings counts the database plus WAL only. Archive summaries separately total referenced downloaded resource bytes, deduplicated by local path. Neither is a complete filesystem usage or backup report. Copying only an active WAL database is not a safe backup design; backup/restore remains unimplemented.

## Korean local search

`storage/archive.rs::archive_query` searches the title and normalized article body, never page navigation/footer text. Whitespace-separated terms are combined with AND; each term may match either title or body. Terms of three or more Unicode characters provide quoted trigram MATCH candidates. Parameterized, escaped literal LIKE checks enforce substring semantics for every term and provide the one/two-character Korean fallback. User punctuation is data, not an FTS query language. Limits are 512 query characters, 32 terms and at most 100 rows per backend page; the UI uses 30.

Results have deterministic publication-date/ID order, source/category filters, saved/unread filters and a nearby plain-text excerpt. `Highlighted.svelte` escapes regex syntax and renders text/`mark` nodes, not HTML snippets returned by SQLite. Search and user-state operations remain available when offline or when the notice extension is disabled. Short-query LIKE scans are an intentional correctness tradeoff, not a claim of benchmarked large-archive performance.

## Source catalog and parsers

`archive/catalog.rs` contains 32 stable identities informed by the owner's public reference catalog. **Catalog presence is not support.** Only these six currently have implemented, verified list-and-detail extraction:

| Source ID | Board |
| --- | --- |
| `sogang-academic` | 서강대학교 학부 학사공지, main API board 2 |
| `computing-academic` | 소프트웨어융합대학 학사 공지 |
| `computing-graduate` | 소프트웨어융합대학 대학원 공지 |
| `computing-external` | 소프트웨어융합대학 대외정보 |
| `computing-updates` | 소프트웨어융합대학 소식 |
| `computing-career` | 소프트웨어융합대학 취업·인턴십 |

The other 26 engineering/department sources are visible as unsupported and cannot be newly subscribed or synchronized. Adding support requires verified article extraction and fixtures, not just a working list URL.

`parser.rs` validates board/article identity, expected metadata, dates, pagination and response structure. The main board uses public paginated JSON list/detail endpoints; HTTP 200 HTML errors are not accepted as JSON articles. Computing uses scoped desktop `.board-list table` rows so duplicated mobile pins do not distort pagination, and extracts the actual `article.board-detail` body. Computing's attachment metadata is read from a restricted scalar/alias subset of serialized Nuxt data; no JavaScript is evaluated. Unexpected structure is an explicit source failure, never an empty successful import. Bounded official samples and provenance are in `archive/fixtures/README.md`.

The reference repository's code reuse license is unestablished. This implementation uses independently written Rust logic; catalog identities and observed public contracts are not permission to relicense reference code. Official notices/attachments retain their original rights, including parser fixture content.

## Manual synchronization and lifecycle

`archive/sync.rs::Controller` admits one archive or resource-retry job at a time. Admission checks the enabled extension and supported subscribed sources under the same store lock used by disabling. Sources run sequentially. One source's failure is recorded independently and does not discard completed documents or prevent the next selected source from running.

- **Historical:** begin at persisted `next_page`, follow public pagination, deduplicate pins/article identities, and checkpoint only after article retrievals on that page succeed. Existing text is not fetched again unnecessarily, but pending resources are resumed. Cancellation retains committed text/files and the last safe page. `history_complete` becomes true only after the last public page is reached; repeated/empty unexpected pages and safety/test limits do not claim completion.
- **Incremental:** start at page one, recheck recent pages and pins with saved ETag/Last-Modified validators where available, and create revisions only for meaningful changes. The minimum recheck window is three pages. New-record discovery continues beyond that window until a page of known regular records establishes the archive frontier; repeated pins do not establish it. This prevents a burst larger than three pages from being silently lost.
- **Initial incremental seed:** if the source has no local documents, fetch at most the first three pages as historical records. Do not inflate the new-notice count or mark full history complete. Historical import can later fill the remainder.
- **Scope limit:** incremental sync is not a complete audit of all old edits, and completed historical import is not a recurring whole-history rescan.

Per-source state records attempt/success timestamps, errors, discovered/archived/failed counts, page and completion. Global status reports current source/page/message for UI polling. Failed or unsupported resources are distinct from failed article retrievals: useful text remains readable with `missing_resources`. Completing source pagination does not promise that every referenced resource was downloaded.

`sogang_notices` is now operational (`available: true`), but opt-in remains separate from source subscriptions. Enabling/restoring preferences does not start network work. The synchronous extension `stop` signals cancellation; the owning async disable command waits for the job to settle without retaining the store lock. Exit similarly cancels jobs and waits before shutting down the AI worker. Disabling hides source controls, rejects new downloads and keeps all archive/search/read/saved functionality and subscription preferences. There is no scheduler, notification queue, tray process, dynamic extension loader, marketplace or external plugin ABI.

## Transport, resources and safe rendering

`archive/network.rs` uses native Reqwest/Rustls. Automatic requests require exact approved HTTPS hosts/routes on port 443. URLs with credentials, fragments or unsafe encoded path separators are rejected. DNS answers must be public and are pinned for the connection; proxies and automatic redirects are disabled. At most three redirects are followed manually, with the same validation. This narrow download policy is separate from resource/link metadata and from explicit browser navigation. Safe HTTP(S) resource references are retained as metadata, so an old HTTP image does not discard otherwise useful article text. Automatic HTTP or unverified-host downloads remain unsupported; a storage regression covers mixed-content metadata and rejects credential-bearing URLs.

Requests are paced at least 650 ms apart. Source `robots.txt` is checked before collection; the most applicable allow/disallow rules and supported crawl delays are applied, with delays above 60 seconds rejected for manual handling. Robots requests use a plain-text Accept header; Computing otherwise returns HTTP 406. There are at most three attempts for transient errors, short bounded backoffs, connect/request/attempt deadlines and cancellable waits/reads. HTTP 429 stops with an actionable message rather than hammering the site. No login, access restriction or anti-bot bypass is used.

### TLS chain completion

The main university host omits its intermediate chain. `archive/tls.rs` supplies two bundled public **untrusted intermediate candidates only for exact `www.sogang.ac.kr`**, then delegates certificate and handshake-signature checks to WebPKI. The trusted root store remains exactly Mozilla's `webpki_roots`; hostname, validity, signature, usage and trusted-root path checks remain mandatory. Computing receives no completion candidates. There is no invalid-certificate acceptance, new root, system-trust change or automatic CA download. Certificate provenance/fingerprints and fixed-time negative tests are documented in `archive/certificates/README.md`.

### Resource ownership

Limits are 4 MiB per list/article response, 10 MiB per inline image, 20 MiB per attachment and 256 MiB of downloaded resources per job. Robots responses are capped at 64 KiB; production pagination has an explicit 100,000-page safety ceiling. Developer test budgets are reported as `limited`, never complete history.

Resource URLs, original filenames, kind, MIME, bytes, state and SHA-256 are retained. Files use generated `resources/<sha256>.<validated-extension>` paths, not remote filenames. Writes go through a temporary `.part` file before the completed path is referenced in SQLite. Raster images require PNG/JPEG/GIF/WebP signatures; attachment extensions are restricted, active web types are rejected, and PDFs require a PDF signature. This is format/transport validation, not antivirus scanning. Files are never automatically opened or executed.

Local access canonicalizes containment, bounds size and verifies the stored SHA-256. A retry repairs corrupt existing bytes; a failed refresh preserves a verified last-known-good copy. Failed, oversized or unsupported resources retain explicit states and can be retried by user action. There is no remote-image fallback and no guarantee that external, deleted, restricted or oversized resources can be archived. Cleanup of unreferenced/partial files and total-storage quotas are deferred.

The Rust sanitizer keeps a limited structural HTML subset, strips scripts/iframes/forms/styles/event attributes and emits image resource references without `src`. `DocumentReader.svelte` reconstructs a second restricted DOM, requests only integrity-checked local raster data URIs, and intercepts HTTP(S) links for explicit OS-browser navigation. Missing/corrupt images become placeholders; known local-integrity failures override persisted download labels and expose repair actions. The original fetched representation is never rendered directly. CSP remains a further layer, not the sanitizer's replacement.

## AI worker: unchanged archive boundary

The main process locates the bundled worker beside its executable, launches it inside Tokio, pipes stdin/stdout, drains stderr and suppresses a separate Windows console. The child is kill-on-drop. Worker failure changes AI status without removing archive/settings access. Settings polls health every five seconds; there is no autonomous restart supervisor.

Protocol version 1 remains serial, ID/version-matched UTF-8 JSON lines over stdin/stdout:

```json
{"version":1,"id":1,"method":"status"}
{"version":1,"id":1,"ok":true,"result":{"engine":"mistral.rs","runtime_available":true,"model":null}}
```

Methods are `ping`, `status`, `infer`, and `shutdown`. Requests have 1 MiB framing limits and validated local model paths, prompt lengths and output-token limits. Lifecycle requests have eight-second deadlines; failed refresh kills/reaps the worker and normal exit requests clean shutdown before force termination. See [`crates/ai-worker/README.md`](../crates/ai-worker/README.md).

mistral.rs stays only in the worker. CPU is the default; Metal/CUDA are optional. Startup does not load weights. Explicit tests use local GGUF paths with no remote token/model resolution. Protocol stdout is isolated from upstream diagnostic prints. v0.2.0 does not add notice inference, model installation, summaries or AI-generated metadata. Real Qwen3-0.6B-Q8_0 CPU generation and loaded-model shutdown were verified in the foundation on macOS Apple Silicon; Qwen3.5 and GPU support remain unverified.

## Packaging and verification boundaries

`src-tauri/tauri.conf.json` bundles the target-specific worker as `externalBin`. `scripts/desktop.mjs` builds/stages the host worker and invokes Tauri; it is not a general cross-compilation helper. macOS has a Korean app/edit/window menu; Windows/Linux use native window controls without unsupported macOS hide actions.

The foundation [CI run](https://github.com/Hiyabye/Rhythm/actions/runs/40083118153) passed on macOS, Ubuntu and Windows. That does not establish v0.2 results. Deterministic tests cover migration rollback/preservation, FTS, parser failures, source isolation, revisions, cancellation, frontier discovery, resource resume/integrity/repair and single-job admission. Locked workspace tests passed with the real worker configured (46 application tests plus five worker tests; three opt-in tests ignored), as did strict workspace Clippy, formatting and frontend checks/build. The normal application crate's explicitly enabled bounded live test retrieved six announcements and nine resources (5,764,144 bytes), then verified local text/search/user state/resource integrity after dropping the network client. The macOS debug app and DMG built; native interaction verified source subscription/import/cancellation, two imported notices, reader text/local image, Korean search, saved/read persistence, extension disabling during collection, retained saved access and worker cleanup on quit. Full historical coverage, complete accessibility testing, release packaging and v0.2 cross-platform execution remain separate gates in [the roadmap](ROADMAP.md). The separate foundation Dependabot run failed to update Linux's transitive glib 0.18.5 for GHSA-wrw7-89jp-8q8g because the GTK/WebKit dependency graph cannot resolve fixed glib 0.20.0. This remains a Linux distribution gate, not a failure of the passing desktop matrix; no reachability assessment or incompatible forced upgrade is implied. No installer signing, updater or publication is implied.
