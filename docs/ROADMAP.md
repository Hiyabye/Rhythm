# Roadmap

Version numbers describe product milestones, not promises that future functionality exists. Rhythm stays local-first and Korean-first throughout. Implementation, automated checks, bounded live collection and native/platform verification are separate claims.

## v0.1.0 — Foundation

**Status: foundation implemented; release/distribution verification remains open.**

The foundation introduced Tauri 2/Rust, Svelte 5/TypeScript, five navigation sections, Pretendard/Beat branding, light/dark/system themes, persistent SQLite settings, an extension registry and an isolated mistral.rs CPU worker. The notice extension was registered but unavailable; archive ingestion did not exist in v0.1.0.

Verified:

- [x] Frontend checks/build and Rust formatting, tests and strict Clippy on macOS Apple Silicon.
- [x] macOS debug `.app` build/launch and bundled worker placement.
- [x] Theme, extension and energy preferences persisted through SQLite and app restarts.
- [x] Quit/window-close cleaned up the worker; worker termination updated AI status without closing the app.
- [x] Real Qwen3-0.6B-Q8_0.gguf CPU generation and loaded-model shutdown with an existing local model.
- [x] [CI run 40083118153](https://github.com/Hiyabye/Rhythm/actions/runs/40083118153) passed the macOS, Ubuntu 24.04 and Windows jobs. This is foundation build/check evidence, not v0.2 or interactive runtime evidence.

Still open: complete accessibility/responsive verification, Windows/Linux interactive behavior, release-optimized installers and model/backend coverage. Metal compilation was blocked by the missing Apple Metal Toolchain. Qwen3.5 2B/4B/9B and CUDA/GPU execution remain unverified. No model weights were downloaded by this implementation.

## v0.2.0 — Local university notice archive

**Status: native archive implementation present; final production-path and release verification remain open.**

Implemented:

- Native Rust collection inside the existing app; no Node collector runtime or archive service.
- A 32-source Sogang catalog with persisted subscriptions. Only **학부 학사공지** and the five **소프트웨어융합대학** boards—학사 공지, 대학원 공지, 대외정보, 소식, 취업·인턴십—are supported. The remaining 26 cannot be subscribed until list-and-detail extraction is verified.
- Transactional schema 2 preserving released schema-1 settings/preferences. Sources/checkpoints, canonical documents, original response snapshots, sanitized/normalized content, revisions, resources and independent read/saved state.
- Manual paginated historical collection, safe checkpoint/resume, repeated-pin deduplication and honest incomplete/limited states.
- Incremental new-record discovery past a minimum three-page edit-recheck window, until a known regular-record frontier. Initial incremental collection seeds recent pages as historical; it does not claim full history or count them as newly published notices.
- Per-source progress/failure isolation, validators, meaningful-change revisions and preservation of old records/user state on failure.
- Paced, cancellable HTTPS collection with robots checks, exact-host/path/DNS/redirect constraints, bounded retries/timeouts and response/resource limits.
- Exact-`www.sogang.ac.kr` missing-chain completion using untrusted intermediate candidates and normal Mozilla/WebPKI verification; no trust-root expansion or TLS bypass.
- Generated local resource paths, size/MIME/hash metadata, integrity-checked image reading/folder reveal, explicit retry/repair and last-good-copy preservation.
- Local title/body Korean substring search with FTS5 trigram candidates and literal short-query fallback, mixed terms, snippets, filtering, deterministic sorting and pagination.
- Real archive counts, inbox/reader/revision/saved-item/search UI, grouped source settings and clear missing-resource/unsupported-source states.
- Operational extension lifecycle: no automatic collection on opt-in/startup; disabling cancels/waits for work and hides source controls without removing archives/subscriptions. Offline archive access remains available.
- Process-lifetime OS data-directory lock before interrupted-job recovery; one application owner and one archive/retry job at a time.

### Verification evidence so far

- [x] Frontend `npm run check` and `npm run build` passed, including the reader recovery controls.
- [x] Integrated deterministic synchronization suite: **16 passed**, one live test intentionally ignored (`cargo test -p rhythm archive::sync -- --nocapture`). Strict application Clippy passed at that checkpoint.
- [x] Deterministic historical test imports **62 documents** across two pages with repeated pins, without duplicates or spurious unchanged revisions.
- [x] Incremental regression archives a burst across more than three pages and reaches **42 total documents**; known pins do not falsely terminate discovery.
- [x] Tests cover cancellation/checkpoint resume, disabling during in-flight retrieval, source failures, validator checks, independent user state, resource interruption/retry and corrupt/oversized-file repair.
- [x] Final integrated **locked workspace tests** passed with the real worker configured: **46 application tests, three worker unit tests and two worker process tests**. Two live application tests and actual-model inference were deliberately ignored in that ordinary suite. Strict locked workspace Clippy, Rust formatting and frontend checks/build passed.
- [x] The explicitly enabled bounded live test in the **normal application crate** retrieved one article from each supported source: **6 documents, 9 downloaded resources, 5,764,144 local resource bytes, zero missing resources**. After dropping the client, reopening the DB verified stored text/search/user state and resource hashes. Every source stayed `limited`, at page one, with `history_complete=false`.
- [x] Schema-1 migration preserves theme, energy, interests and extension preferences; schema-2 reopen, newer-schema rejection and migration rollback pass. Native startup also upgraded the existing schema-1 application DB while retaining light/eco/extension preferences.
- [x] Safe HTTP resource metadata no longer rejects useful article text; the automatic HTTPS-only download policy remains unchanged, and credential-bearing metadata is rejected.
- [x] macOS debug `.app` and `.dmg` built and the app/worker launched. Native checks exercised source subscription, a bounded two-notice import, cancellation, reader text/local image, Korean search/highlighting, read/save state, extension disabling during resumed collection, retained saved access after restart and normal worker cleanup on quit.

Computing's robots endpoint requires `Accept: text/plain,*/*;q=0.1`; the earlier HTML-oriented header returned HTTP 406. This fix is present in the production transport exercised by the integrated live test. Live article counts and files may change on later runs.

### Remaining verification gates

- [ ] Verify keyboard/screen-reader behavior, reader links, error recovery, long articles/tables and narrower layouts in native webviews.
- [x] Observed [v0.2.0 CI run 40110222263](https://github.com/Hiyabye/Rhythm/actions/runs/40110222263): macOS, Windows and Ubuntu desktop jobs passed at `6465b00`. Windows/Linux interactive runtime and file reveal remain unverified.
- [ ] Resolve the existing Linux transitive `glib 0.18.5` advisory before Linux distribution. Separate [Dependabot run 40090952341](https://github.com/Hiyabye/Rhythm/actions/runs/40090952341), against foundation commit `e60e6a0`, failed with `security_update_not_possible`: latest resolvable 0.18.5 versus first fixed 0.20.0. [GHSA-wrw7-89jp-8q8g / RUSTSEC-2024-0429](https://github.com/advisories/GHSA-wrw7-89jp-8q8g) describes unsound `VariantStrIter` iteration and possible crashes. The current Linux dependency tree reaches it through Tauri/GTK/WebKit; forcing glib 0.20 is not a compatible lockfile-only update. No exploitation/reachability claim was established, no platform was removed, and this is not a desktop matrix build failure.
- [ ] Import complete accessible historical coverage from the priority sources and inspect source-specific failures/checkpoints. **No complete live historical import has been performed.** Fixture pagination and one live article per source do not establish it.
- [ ] Verify release-optimized installers/signing before distribution claims.

Limits are intentional: not every external/deleted/restricted/oversized attachment can be stored; source-history completion describes traversed public pages, not guaranteed resource completeness. Incremental checks do not audit every old edit. Archive garbage collection, full backup/restore and total-storage quotas are not implemented.

No AI notice processing, notifications, scheduler, automatic background collection, external web crawling or cloud service is added in v0.2.0. The AI worker remains separate and its protocol unchanged.

## v0.2.1 — Refinement, reliability & hardening

**Status: stabilization implemented and locally verified; Linux distribution and untested platforms remain gated.** Architecture and schema 2 are preserved; released migrations are unchanged. This milestone does not add notice intelligence or automatic collection.

Implemented corrections:

- Computing news public/internal attachment IDs validated separately; inline-image roles retained when URLs also occur as attachments. Existing attachment-role raster archives remain readable without data rewriting.
- Main API consistency/duplicate checks and a current native Engineering SSR parser. Four existing catalog IDs gain verified support: `eng-general`, `eng-careers`, `eng-academic`, `eng-research`. Total: **32 cataloged / 10 supported / 22 unsupported**. See [collector comparison](COLLECTOR_COMPARISON.md) for code evidence, rejected approaches and remaining families.
- HTTP 304 resumes pending resources; explicit failed resource retries no longer report success. Completed resource responses consume the job byte budget even if validation/writes fail. Interrupted text/resource work and last-good local files remain intact.
- Robots wildcard/end-anchor matching, malformed delay/error-page rejection and policy validation before **every redirect hop**. Cross-origin redirects are rejected by the source policy. Normal TLS/Mozilla-root verification is unchanged; only approved narrow Engineering routes were added.
- Progress counters are **per attempt**, avoiding historical retry double-counting; checkpoints remain cumulative. UI distinguishes never imported, incomplete, interrupted and completed history, and historical versus incremental work.
- Search rejects embedded NUL with Korean validation instead of false matches/FTS errors. Selected-revision snippets use the selected body; resource reuse requires matching URL and role.
- Reader content remains available if automatic read-state persistence fails. Filtered pagination recovers when its last item disappears. Error states do not claim a successfully empty archive. Pending writes, refresh generations, late read-state writes and return focus are reconciled.
- Render-local image reads are deduplicated; obsolete rendering stops admitting more reads. Wide content is keyboard-focusable/scrollable, scientific sub/sup formatting survives, light semantic text contrast is strengthened, and raw storage diagnostics receive actionable Korean feedback.
- Worker status payloads are typed/validated; malformed responses terminate/reap the child. stderr drains bounded chunks even without newlines/valid UTF-8. Process tests have kill/reap deadlines. A second data-directory owner exits normally rather than triggering Tauri's setup panic.

### v0.2.1 evidence

- [x] Before changes: frontend checks/build, formatting, strict Clippy and **51 Rust tests** passed with the real worker configured. Temporary probes reproduced defects before fixes; committed regressions cover them where practical.
- [x] Final locked workspace tests: **79 application + 3 worker unit + 3 worker process = 85 passed**. Four opt-in tests were ignored in this ordinary suite: two live probes, the benchmark and real-model inference. The actual worker lifecycle/crash-isolation test ran with `RHYTHM_WORKER_TEST_BIN` set.
- [x] Rustfmt, strict locked workspace Clippy, Svelte diagnostics (**zero errors/warnings**) and frontend production build passed. macOS debug linking emits a nonfatal upstream-heavy worker `__eh_frame` compact-unwind size warning; Rust lint checks remain clean. Release-optimized behavior was not inferred from this debug build.
- [x] **38 Playwright checks** passed: 19 cases on Chromium and WebKit, using compiled Svelte and an explicitly mocked IPC boundary. They cover navigation/settings rollback/system themes, subscription/loading/error/cancel controls, saved/unread pagination, stale responses, reader read-write failures, local images/repair feedback, safe markup, focus, 640 px/200% layout and contrast. These are frontend integration tests, not proof of real Tauri/SQLite integration.
- [x] Released schema-2 fixture/reopen, schema-1 migration/rollback, late revision-trigger rollback, SQLite-full page-cap failure, FK/SQLite/FTS integrity, literal punctuation/limits, deterministic equal-date pagination and metadata revision tests passed. No migration was added or rewritten. Resource filesystem write failure is separately tested without filling the host disk.
- [x] Reproducible ignored **5,000-document** benchmark passed through real Store APIs. Seven warm debug samples: inbox median **10.57 ms**, deep pagination **16.11 ms**, Korean 1/2-character **17.15/16.97 ms**, broad 3-character **32.56 ms**, mixed Korean/English **51.72 ms**, source/category **8.73 ms**. A 152,007-character document read took **1.83 ms**, revision/FTS update **8.35 ms**. These are this host's observations, not release/IPC/cold-cache/p95 claims; no speculative index/search-engine replacement was justified.
- [x] Bounded production Rust live/offline test: **10 articles, 17 resources, 7,294,862 bytes, zero missing resources**, one article per supported source. All ten sources remained deliberately incomplete. Local text/search/user state/resource hashes were verified after dropping the client and reopening SQLite.
- [x] Isolated native macOS fresh-launch workflow: enable/subscribe, cancel Engineering academic import after **two articles**, resume to its observed **five-record public endpoint**, open article/attachments, save/read, Korean search, restart, retain dark theme/state, disable extension and keep saved access. That board had six downloaded resources and one unsupported legacy reference: article and resource completeness remained separate.
- [x] Native processes under a network-denying macOS sandbox read/search existing archives and rendered a local image. Deliberate corruption affected only a temporary archive copy: placeholder/integrity feedback, truthful failed offline retry and explicit successful online repair were observed. A legacy attachment-role inline image rendered through real IPC. Folder reveal, focusable reader, narrow native layout/light/dark themes, worker failure without app failure, no app/worker TCP listener, normal quit/worker cleanup and second-owner rejection were checked. These are targeted checks, not full VoiceOver conformance.
- [x] Native macOS v0.2.1 debug `Rhythm.app` built and ran. No v0.2.1 DMG/release installer/signing/publishing is claimed.
- [x] Reviewed actual **v0.2.0** three-OS CI success and separate [Dependabot failure 40146725909](https://github.com/Hiyabye/Rhythm/actions/runs/40146725909). Added a dedicated Chromium/WebKit CI job; **v0.2.1 remote CI has not run** because nothing was pushed.
- [x] `npm audit`: zero reported vulnerabilities. Direct dependency license metadata was reviewed (MIT/Apache/ISC, WebPKI roots CDLA-Permissive, Pretendard OFL); no incompatible direct license was identified. The repository's open Dependabot alert is glib. This is not a complete transitive Rust advisory/license audit.

### Remaining gates and decisions

- **Linux distribution blocked:** glib 0.18.5 / RUSTSEC-2024-0429 remains in the GTK3/WebKit/Tauri graph. Dependabot still resolves at most 0.18.5, below fixed 0.20.0. [Upstream fix #1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343) and [Debian's 0.18.5 backport](https://sources.debian.org/patches/rust-glib-0.18/0.18.5-7/) were reviewed. The user chose to retain the distribution gate rather than vendor an unverified third-party patch. No forced upgrade, audit suppression or security-check removal was made. No claim of exploitability or absence of reachable affected code is established.
- **Platform/accessibility:** Windows/Linux native interaction and v0.2.1 remote CI are unverified. Browser WebKit is not all native webviews; full VoiceOver navigation, release installers and signing remain open. No upstream-heavy worker unwind performance claim is made.
- **External data:** complete historical coverage across all supported sources remains unverified. Only the bounded ten-source sample, deterministic multi-page imports, and one small live Engineering board's observed endpoint were verified. Unsupported families stay unavailable. Deleted/changed external websites and unsafe/unsupported/missing files remain explicit failures.
- **Resource policy:** URL-and-role reuse does not detect overwritten binary bytes at an unchanged URL. Explicit retry can update a revision's resource reference; immutable binary revisioning is not promised. Unreferenced/partial-file cleanup, total quotas and full backup/restore remain deferred.
- **v0.3 readiness:** local archive correctness regressions and known fixable issues identified here are addressed. The platform/distribution gates remain mandatory before those release claims; they do not prevent local intelligence development on the verified macOS foundation. GPU/toolchain/model validation remains separate.

## v0.3.0 — Local AI model management

Planned: compatible model selection/downloads, first-run setup, explicit storage/resource information, local summaries/classifications and lazy processing. Validate exact Qwen3.5 artifacts and hardware backends before claiming support. Keep weights separate from application installers.

## v0.4.0 — Personalization

Planned: user-controlled persistent memory, explicit preferences/feedback, explainable relevance ranking and personalized recommendations. Existing interest text is stored only; automatic learning/ranking is not implemented.

## v0.5.0 — Proactive operation

Planned: background execution, native notifications, scheduling and energy-aware processing. Apply energy profiles to frequency, residency and background work. Disabling extensions must cancel their work/notifications without deleting data.

## v0.9.0 — Integration and distribution readiness

Planned: integration testing, usability refinement, installer/updater work, resource/error recovery validation and distribution readiness. GitHub Releases and Tauri distribution are the intended direction; publishing/updating is not an operational service today.

## v1.0.0 — Stable release

Planned: stable local-first daily command-center experience with validated platform behavior, reliable archives/AI integration, clear privacy controls and supported distribution.

## After v1.0.0

Possible built-in extensions:

- **Cybercampus:** announcements, assignments, courses and deadlines; port appropriate functionality from `rycont/eclass-cli` into a native Rust connector.
- **Opportunities:** competitions, extracurricular activities, internships, research opportunities and jobs.
- **Calendar and Tasks:** schedules, deadlines, reminders and daily planning.
- **Email:** defer until OAuth and provider restrictions can be handled deliberately.
- **Personal Agent:** context-aware proposals and workflows whose consequential actions remain under user control.

These extensions should share the archive, search and personalization core. No marketplace, cloud synchronization or autonomous external-action infrastructure is part of the current release.
