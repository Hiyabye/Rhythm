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

There are only two Cargo workspace members: the Tauri app and worker. v0.2 adds the archive inside the existing main process, not another service or a transplanted Node collector. v0.2.1 hardens those boundaries and adds the current Engineering SSR parser, without changing schema 2. Notice collection never calls the AI worker. Generic records and storage remain separate from source-specific route/HTML extraction.

## Application ownership and commands

`src/App.svelte` owns navigation, settings and theme application. `src/lib/ArchiveView.svelte` handles archive lists/search; `DocumentReader.svelte` reads a stored revision; `SourceSettings.svelte` manages subscriptions and manual jobs. `src/lib/types.ts` mirrors Rust command payloads.

Foundation commands remain `get_snapshot`, `save_settings`, `set_extension`, and `worker_status`. `src-tauri/src/archive/mod.rs` adds:

- `archive_sources`, `archive_subscribe`, `archive_summary`: catalog, subscriptions and actual local counts.
- `archive_query`, `archive_document`, `archive_state`: search/list pages, a current or prior revision, and independent read/saved state.
- `archive_start`, `archive_cancel`, `archive_status`: admit, cancel and observe a manual job.
- `archive_retry_resource`, `archive_resource_image`, `archive_reveal_resource`: explicit downloads and integrity-checked local access.
- `archive_open_original`: explicit HTTP(S) navigation through the OS browser; no file, script or custom-protocol URL execution.

`AppState` owns an `Arc<Mutex<Store>>`, the archive controller/client, the extension registry, and a Tokio-mutex-protected AI worker. SQLite guards are not held across network or worker awaits. Search runs through `spawn_blocking`; frontend generation checks prevent stale search/reader responses from replacing a newer request.

Before `Store::open` can recover interrupted work, `core::lock_data_dir` obtains a process-lifetime OS file lock on `app_data_dir()/rhythm.lock`. A second owner of that data directory is rejected with a Korean diagnostic and normal exit before opening the store/worker; Tauri's setup-error panic is avoided. The OS releases the lock after process termination; startup does not infer ownership from an old file's mere existence. This complements the controller's in-process single-job admission and prevents concurrent instances from racing checkpoints or temporary resource files.

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

`Store::upsert_article` commits the current document, revision, user-state initialization and resource references transactionally. Identity includes the source, so equal article IDs on different boards do not collide. Hash input includes canonical URL, title, author, category, publication date, sanitized body/text and resource identities—not volatile view counts, collection time or raw response churn. An unchanged check updates validators/check time without another revision. Revisions never replace read/saved state. Old snapshots and their resource references remain selectable. Downloaded resources are reused only for the same document, URL and role. This does not detect a remote binary overwritten at an unchanged URL; explicit retry can replace that revision's resource reference. Immutable binary revisioning is not claimed.

Original responses are stored as revision data, not injected into the webview. A parse, network or write failure does not replace the last good article with an error page. There are no deletion/tombstone semantics or automatic retention cleanup in v0.2.0.

Database size in Settings counts the database plus WAL only. Archive summaries separately total referenced downloaded resource bytes, deduplicated by local path. Neither is a complete filesystem usage or backup report. Copying only an active WAL database is not a safe backup design; backup/restore remains unimplemented.

## Korean local search

`storage/archive.rs::archive_query` searches the title and normalized article body, never page navigation/footer text. Whitespace-separated terms are combined with AND; each term may match either title or body. Terms of three or more Unicode characters provide quoted trigram MATCH candidates. Parameterized, escaped literal LIKE checks enforce substring semantics for every term and provide the one/two-character Korean fallback. User punctuation is data, not an FTS query language. Embedded NUL is rejected before constructing SQLite LIKE/FTS input; tabs/newlines remain term separators. Limits are 512 query characters, 32 terms and at most 100 rows per backend page; the UI uses 30.

Results have deterministic publication-date/ID order, source/category filters, saved/unread filters and a nearby plain-text excerpt. `Highlighted.svelte` escapes regex syntax and renders text/`mark` nodes, not HTML snippets returned by SQLite. Search and user-state operations remain available when offline or when the notice extension is disabled. Short-query LIKE scans are an intentional correctness tradeoff. The reproducible ignored 5,000-record benchmark measures correctness and warm debug timings rather than enforcing fragile speed thresholds; observed medians were about 11 ms for inbox, 17 ms for short Korean and 52 ms for broad mixed queries. These do not establish release/native-IPC/cold-cache performance; see the roadmap for the workload and measurements.

## Source catalog and parsers

`archive/catalog.rs` contains 32 stable identities informed by the owner's public reference catalog. **Catalog presence is not support.** These ten currently have implemented list/detail extraction and bounded live local-resource/offline verification:

| Source ID | Board |
| --- | --- |
| `sogang-academic` | 서강대학교 학부 학사공지, main API board 2 |
| `computing-academic` | 소프트웨어융합대학 학사 공지 |
| `computing-graduate` | 소프트웨어융합대학 대학원 공지 |
| `computing-external` | 소프트웨어융합대학 대외정보 |
| `computing-updates` | 소프트웨어융합대학 소식 |
| `computing-career` | 소프트웨어융합대학 취업·인턴십 |
| `eng-general` | 공과대학 일반공지, `/board/notice` |
| `eng-careers` | 공과대학 취업정보, `/board/recruit` |
| `eng-academic` | 공과대학 학사공지, `/board/academic` |
| `eng-research` | 공과대학 연구공지, `/board/research` |

The other 22 newsletter/department sources are visible as unsupported and cannot be newly subscribed or synchronized. Adding support requires verified article extraction and fixtures, not just a working list URL.

`parser.rs` validates board/article identity, expected metadata, dates, pagination and response structure. The main board uses public paginated JSON list/detail endpoints; HTTP 200 HTML errors are not accepted as JSON articles. Computing uses scoped desktop `.board-list table` rows so duplicated mobile pins do not distort pagination, and extracts the actual `article.board-detail` body. Computing's attachment metadata is read from a restricted scalar/alias subset of serialized Nuxt data; no JavaScript is evaluated. Public news route IDs can differ from internal `DETAIL.SN`: canonical/hydrated route, internal file parent and public board/file path must all agree. Inline image roles take precedence when attachments share their URL. `engineering.rs`, a source-specific parser submodule, validates current SSR canonical/form/board/pagination identity, regular ordinals, full title/body, explicit attachment names and visible Korean time against machine UTC. Main JSON listings validate size/total/page/pin/duplicate consistency. Unexpected structure is an explicit source failure, never an empty successful import. Bounded official samples and provenance are in `archive/fixtures/README.md`.

Both reference repositories' code reuse licenses are unestablished; see the snapshot-based [collector comparison](COLLECTOR_COMPARISON.md). This implementation uses independently written Rust logic; catalog identities and observed public contracts are not permission to relicense reference code. Official notices/attachments retain their original rights, including parser fixture content.

## Manual synchronization and lifecycle

`archive/sync.rs::Controller` admits one archive or resource-retry job at a time. Admission checks the enabled extension and supported subscribed sources under the same store lock used by disabling. Sources run sequentially. One source's failure is recorded independently and does not discard completed documents or prevent the next selected source from running.

- **Historical:** begin at persisted `next_page`, follow public pagination, deduplicate pins/article identities, and checkpoint only after article retrievals on that page succeed. Existing text is not fetched again unnecessarily, but pending resources are resumed. Cancellation retains committed text/files and the last safe page. `history_complete` becomes true only after the last public page is reached; repeated/empty unexpected pages and safety/test limits do not claim completion.
- **Incremental:** start at page one, recheck recent pages and pins with saved ETag/Last-Modified validators where available, and create revisions only for meaningful changes. The minimum recheck window is three pages. New-record discovery continues beyond that window until a page of known regular records establishes the archive frontier; repeated pins do not establish it. This prevents a burst larger than three pages from being silently lost.
- **Initial incremental seed:** if the source has no local documents, fetch at most the first three pages as historical records. Do not inflate the new-notice count or mark full history complete. Historical import can later fill the remainder.
- **Scope limit:** incremental sync is not a complete audit of all old edits, and completed historical import is not a recurring whole-history rescan.

Per-source state records attempt/success timestamps, errors, per-attempt discovered/archived/failed counts, cumulative page and completion. Counters reset for each historical or incremental attempt, so retried failed IDs are not double-counted as lifetime discoveries. HTTP 304 updates article check time and still resumes its pending resources. A resource outcome distinguishes recorded nonfatal download failure from success; normal article ingestion can retain text while explicit retry reports the actual failure. Global status reports current source/page/message for UI polling. Failed or unsupported resources are distinct from failed article retrievals: useful text remains readable with `missing_resources`. Completing source pagination does not promise that every referenced resource was downloaded.

`sogang_notices` is now operational (`available: true`), but opt-in remains separate from source subscriptions. Enabling/restoring preferences does not start network work. The synchronous extension `stop` signals cancellation; the owning async disable command waits for the job to settle without retaining the store lock. Exit similarly cancels jobs and waits before shutting down the AI worker. Disabling hides source controls, rejects new downloads and keeps all archive/search/read/saved functionality and subscription preferences. There is no scheduler, notification queue, tray process, dynamic extension loader, marketplace or external plugin ABI.

## Transport, resources and safe rendering

`archive/network.rs` uses native Reqwest/Rustls. Automatic requests require exact approved HTTPS hosts/routes on port 443. URLs with credentials, fragments or unsafe encoded path separators are rejected. DNS answers must be public and are pinned for the connection; proxies and automatic redirects are disabled. At most three same-origin redirects are followed by the sync policy layer, with static URL/DNS/TLS checks and robots authorization before each hop. Transport returns redirect targets rather than following them implicitly. Only the exact approved main, Computing and Engineering HTTPS routes are eligible. This narrow download policy is separate from resource/link metadata and from explicit browser navigation. Safe HTTP(S) resource references are retained as metadata, so an old HTTP image does not discard otherwise useful article text. Automatic HTTP or unverified-host downloads remain unsupported; a storage regression covers mixed-content metadata and rejects credential-bearing URLs.

Requests are paced at least 650 ms apart. Source `robots.txt` is checked before collection; the most applicable allow/disallow rules and supported crawl delays are applied, with delays above 60 seconds rejected for manual handling. Robots requests use a plain-text Accept header; Computing otherwise returns HTTP 406. HTML error pages and malformed/nonfinite/negative/over-60-second crawl delays fail closed. Wildcard/end-anchor matching considers later possible matches instead of permitting a disallowed path after a greedy first occurrence. There are at most three attempts for transient errors, short bounded backoffs, connect/request/attempt deadlines and cancellable waits/reads. HTTP 429 stops with an actionable message rather than hammering the site. No login, access restriction or anti-bot bypass is used.

### TLS chain completion

The main university host omits its intermediate chain. `archive/tls.rs` supplies two bundled public **untrusted intermediate candidates only for exact `www.sogang.ac.kr`**, then delegates certificate and handshake-signature checks to WebPKI. The trusted root store remains exactly Mozilla's `webpki_roots`; hostname, validity, signature, usage and trusted-root path checks remain mandatory. Computing receives no completion candidates. There is no invalid-certificate acceptance, new root, system-trust change or automatic CA download. Certificate provenance/fingerprints and fixed-time negative tests are documented in `archive/certificates/README.md`.

### Resource ownership

Limits are 4 MiB per list/article response, 10 MiB per inline image, 20 MiB per attachment and 256 MiB of completed resource response bytes per job. Received response bytes are charged before validation/disk persistence and the next response is capped to remaining budget. Bytes from incomplete transport failures/retries are not exposed by the transport contract and are not included in that counter. Robots responses are capped at 64 KiB; production pagination has an explicit 100,000-page safety ceiling. Developer test budgets are reported as `limited`, never complete history.

Resource URLs, original filenames, kind, MIME, bytes, state and SHA-256 are retained. Files use generated `resources/<sha256>.<validated-extension>` paths, not remote filenames. Writes go through a temporary `.part` file before the completed path is referenced in SQLite. Raster images require PNG/JPEG/GIF/WebP signatures; attachment extensions are restricted, active web types are rejected, and PDFs require a PDF signature. This is format/transport validation, not antivirus scanning. Files are never automatically opened or executed.

Local access canonicalizes containment, bounds size and verifies the stored SHA-256. A retry repairs corrupt existing bytes; a failed refresh preserves a verified last-known-good copy. Failed, oversized or unsupported resources retain explicit states and can be retried by user action. There is no remote-image fallback and no guarantee that external, deleted, restricted or oversized resources can be archived. Cleanup of unreferenced/partial files and total-storage quotas are deferred.

The Rust sanitizer keeps a limited structural HTML subset, strips scripts/iframes/forms/styles/event attributes and emits image resource references without `src`. `DocumentReader.svelte` reconstructs a second restricted DOM, requests only integrity-checked local raster data URIs, and intercepts HTTP(S) links for explicit OS-browser navigation. Missing/corrupt images become placeholders; known local-integrity failures override persisted download labels and expose repair actions. A render-local promise cache validates repeated images only once; disposed/stale renders stop admitting subsequent image reads. Legacy attachment-role inline images are accepted only after the same backend local-integrity/raster/10 MiB checks. A failed automatic read-state write cannot suppress retrieved content. The original fetched representation is never rendered directly. CSP remains a further layer, not the sanitizer's replacement.

## AI worker: unchanged archive boundary

The main process locates the bundled worker beside its executable, launches it inside Tokio, pipes stdin/stdout, drains stderr and suppresses a separate Windows console. The child is kill-on-drop. Worker failure changes AI status without removing archive/settings access. Health requires a typed status payload with the expected engine, boolean runtime availability and explicitly present nullable/string model. Missing/malformed payloads terminate/reap the child rather than reporting healthy. stderr drains fixed 4 KiB chunks even without newlines/valid UTF-8; lifecycle process tests have deadlines and kill/reap on timeout. Settings polls health every five seconds; there is no autonomous restart supervisor.

Protocol version 1 remains serial, ID/version-matched UTF-8 JSON lines over stdin/stdout:

```json
{"version":1,"id":1,"method":"status"}
{"version":1,"id":1,"ok":true,"result":{"engine":"mistral.rs","runtime_available":true,"model":null}}
```

Methods are `ping`, `status`, `infer`, and `shutdown`. Requests have 1 MiB framing limits and validated local model paths, prompt lengths and output-token limits. Lifecycle requests have eight-second deadlines; failed refresh kills/reaps the worker and normal exit requests clean shutdown before force termination. See [`crates/ai-worker/README.md`](../crates/ai-worker/README.md).

mistral.rs stays only in the worker. CPU is the default; Metal/CUDA are optional. Startup does not load weights. Explicit tests use local GGUF paths with no remote token/model resolution. Protocol stdout is isolated from upstream diagnostic prints. v0.2.0 does not add notice inference, model installation, summaries or AI-generated metadata. Real Qwen3-0.6B-Q8_0 CPU generation and loaded-model shutdown were verified in the foundation on macOS Apple Silicon; Qwen3.5 and GPU support remain unverified.

## Packaging and verification boundaries

`src-tauri/tauri.conf.json` bundles the target-specific worker as `externalBin`. `scripts/desktop.mjs` builds/stages the host worker and invokes Tauri; it is not a general cross-compilation helper. macOS has a Korean app/edit/window menu; Windows/Linux use native window controls without unsupported macOS hide actions.

Actual [v0.2.0 CI](https://github.com/Hiyabye/Rhythm/actions/runs/40110222263) passed on macOS, Ubuntu and Windows. v0.2.1 remote CI has not run; the new dedicated Chromium/WebKit job is configured, not observed. Local v0.2.1 evidence includes 85 passing Rust tests with the real worker configured, 38 frontend browser checks, strict Clippy/formatting/frontend build, a macOS debug app, bounded ten-source live collection/offline reopen, and isolated native network-denied reading/search/image integrity/repair and lifecycle checks. Normal tests remain network-free. Opt-in benchmark/live/model tests and the exact native checks are documented separately in [the roadmap](ROADMAP.md); mocked IPC tests do not establish native backend correctness.

Linux distribution remains blocked by glib 0.18.5 / RUSTSEC-2024-0429. The GTK3/WebKit graph cannot resolve fixed glib 0.20.0; the latest separate Dependabot job failed for that constraint. The user chose the explicit distribution gate over a locally vendored upstream backport. Security checks remain enabled; no reachability assessment, forced incompatible upgrade or claim that the advisory is resolved is implied. Complete historical coverage, full VoiceOver use, Windows/Linux native runtime and release-optimized installers/signing remain unverified. No updater or publication is implemented.
