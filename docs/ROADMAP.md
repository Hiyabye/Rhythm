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
- [ ] Observe v0.2 macOS/Windows/Linux CI, then separately test Windows/Linux runtime and file reveal/locking behavior.
- [ ] Resolve the existing Linux transitive `glib 0.18.5` advisory before Linux distribution. Separate [Dependabot run 40090952341](https://github.com/Hiyabye/Rhythm/actions/runs/40090952341), against foundation commit `e60e6a0`, failed with `security_update_not_possible`: latest resolvable 0.18.5 versus first fixed 0.20.0. [GHSA-wrw7-89jp-8q8g / RUSTSEC-2024-0429](https://github.com/advisories/GHSA-wrw7-89jp-8q8g) describes unsound `VariantStrIter` iteration and possible crashes. The current Linux dependency tree reaches it through Tauri/GTK/WebKit; forcing glib 0.20 is not a compatible lockfile-only update. No exploitation/reachability claim was established, no platform was removed, and this is not a desktop matrix build failure.
- [ ] Import complete accessible historical coverage from the priority sources and inspect source-specific failures/checkpoints. **No complete live historical import has been performed.** Fixture pagination and one live article per source do not establish it.
- [ ] Verify release-optimized installers/signing before distribution claims.

Limits are intentional: not every external/deleted/restricted/oversized attachment can be stored; source-history completion describes traversed public pages, not guaranteed resource completeness. Incremental checks do not audit every old edit. Archive garbage collection, full backup/restore and total-storage quotas are not implemented.

No AI notice processing, notifications, scheduler, automatic background collection, external web crawling or cloud service is added in v0.2.0. The AI worker remains separate and its protocol unchanged.

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
