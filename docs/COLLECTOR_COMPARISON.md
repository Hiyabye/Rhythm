# Collector comparison — v0.2.1

Read-only code review at these snapshots; reference tests were **inspected, not executed**. Bounded public-site samples were captured without running reference JavaScript or contacting deployed collectors.

- Rhythm baseline: `6465b0005992c69faf2c5524ed48f229df55557d`.
- [SogangFriends](https://github.com/Hiyabye/SogangFriends/tree/b48a5d736abe335198738c6de54fcf47d274f757): `b48a5d736abe335198738c6de54fcf47d274f757`.
- [sogang-notices](https://github.com/Hiyabye/sogang-notices/tree/5d53d9575c3f92ef2f36d39cecc2c8a406e5be64): `5d53d9575c3f92ef2f36d39cecc2c8a406e5be64`.

Neither reference has an explicit source-code license in the inspected snapshot. Rhythm adopts observed contracts and ideas through independently written Rust, not substantial source copying. Official fixture content retains its original rights. Reference repositories/services were not modified.

## Evidence-backed comparison

Counts describe different products, not interchangeable levels of archive support.

| Responsibility | Rhythm v0.2.0 → v0.2.1 | SogangFriends | sogang-notices |
| --- | --- | --- | --- |
| Catalog / implementation | 32 cataloged, 6 → **10 full-article/local-resource sources** | 26 source paths: 6 university, 5 Computing, 10 legacy CMS, 4 Engineering, 1 Business; broad code-level list/raw support, not independently live-verified for every board here | 32 listing/feed sources; **not full-article archive support** |
| University coverage | Main academic API board 2 | API boards 2/34/141/1/142/3 include academic/student/scholarship/general/events/onestop | Main academic board 2 |
| Engineering | Retired CMS identities retained; four mapped to current SSR boards in v0.2.1 | Current `/board/{notice,recruit,academic,research}` with full articles | Old CMS listings; other department families include PHP and mechanical SSR |
| Identity / extraction | Source + public ID; authoritative detail title/date/author/body; v0.2.1 validates distinct Computing internal attachment parent | Strong public/internal Computing identity separation; source-specific full-title recovery and UTC/KST checks | Strong listing validation; CMS full-title comments and narrow malformed-title handling |
| Pins / pagination | Desktop-scoped pins; durable pages; extended incremental discovery; v0.2.1 strengthens main JSON consistency | Source-specific ordinals/final-page proof and conflict checks; recent HTML collection generally capped at three pages | Explicit pin slot semantics, ordinal/date order, cross-page regular overlap and conflicting-pin checks |
| Article completeness | Original response + sanitized structural HTML/text, tables, local raster images, attachments, explicit incomplete states | Raw body/attachment/image **references**; raw collector deliberately does not fetch binaries | Lists and newest-30 metadata feeds only |
| Revisions / state | Meaningful revisions; independent read/saved state; old local snapshots remain accessible | Raw archive uses existing-ID checks and `INSERT OR IGNORE`: principally first capture, not equivalent revision detection | Rolling feed replacement, no document revision model |
| History / resume | Persistent local checkpoints, cancellation, resources resumed after text commit and 304 rechecks | Bounded backfill with cursor commit after raw capture, missing-ID optimization; explicit unavailable records for 404/410 | “Full” refreshes all feeds, not all historical articles |
| Reliability | Public-DNS pinning, exact HTTPS routes, robots per redirect hop, budgets, verified TLS chain completion, last-good local files | Host restrictions, manual redirects, pacing, bounded streaming; cloud workflow failure isolation | Redirect refusal, pacing/cancellation tests, staged publication preserving last-good feeds |
| Maintainability / fit | Small native modular monolith; SQLite/resources/reader remain local | Useful parser/protocol lessons; Discord/D1/Queues are not a local archive architecture | Useful listing validators/pacing; hosted feeds/batch publication are not a local archive architecture |

### Code pointers

Within the linked immutable snapshots:

- **SogangFriends:** `src/sources.ts`, `university-notice-sources.ts`, `legacy-notice-sources.ts`, `engineering-notice-sources.ts`, `business-notice-sources.ts`; `notice-archive-sources.ts` and `notice-archive.ts`; `scripts/notice-collector.mjs`, `backfill-notices.mjs`, `notice-archive-client.mjs`. Relevant tests: `tests/notice-archive-sources.test.ts` and `engineering-notice-sources.test.ts`.
- **sogang-notices:** all definitions in `sources.mjs`; `collect.mjs`, `list.mjs`, `cms.mjs`, `computing.mjs`, `community.mjs`, `mechanical.mjs`, `semiconductor.mjs`, `html.mjs`, `pacing.mjs`; parser and `test/pacing.test.mjs` coverage.
- **Rhythm:** `src-tauri/src/archive/{catalog,parser,engineering,network,sync}.rs`, official samples in `archive/fixtures/`, synchronization regressions in `archive/sync/tests.rs`; transactional archive/search tests in `storage/archive_tests.rs`.

## Adopted independently

1. **Computing public/internal identity distinction.** Official `/ko/community/news/detail/282` has internal `DETAIL.SN=562`, attachment parent 562, but public file path `community/news/282/`. Rhythm previously rejected it. The Rust parser now requires canonical public identity and hydrated route proof, matches attachment parents to internal SN and file paths to public ID/board, and rejects hostile mismatches. SogangFriends' corresponding regression informed the contract; the captured official article is a deterministic Rust fixture.
2. **Current Engineering SSR family.** Existing `eng-general`, `eng-careers`, `eng-academic`, `eng-research` IDs now map to `notice`, `recruit`, `academic`, `research`. Listing canonical/form/ordinal/pagination proof, detail identity, full titles without badges, attachment names, and machine-UTC versus visible-KST agreement are checked. Sanitization, persistence and downloads reuse Rhythm's generic pipeline. No parallel legacy implementation is introduced.
3. **Fail-closed listing consistency.** Main JSON row count, total/page relationships, pin values, and duplicate identities now fail explicitly instead of silently accepting conflicting/incomplete responses. This adopts the references' validation principle, not newest-30 feed assumptions. Existing main fixtures and the sync fake were updated together to model the real contract.
4. **Pacing/policy boundary tests.** Robots wildcard/end-anchor cases, malformed crawl delays/error HTML, redirect authorization before the next request, resource-response byte accounting and failure-state tests strengthen the local pipeline.

## Retained and rejected

Retain Rhythm's source-qualified document identities, meaningful revisions, independent user state, offline binaries/integrity, explicit source lifecycle and incremental discovery beyond three pages. SogangFriends' fixed recent window and first-capture-only raw optimization would regress this archive. A 304 response now still resumes pending local resources.

Do not transplant Node collectors, Discord, hosted feeds, D1, queues, cloud storage or scheduled batches. Do not call sogang-notices' “full” feed refresh a historical import. Do not equate list extraction or remote file references with an offline archive. Do not silently adopt 404/410 skipping: deletion/tombstone policy remains unimplemented, and an unresolved article failure must not advance a historical checkpoint.

Additional university API boards and legacy/PHP/mechanical families remain candidates, not an invitation to grow this stabilization milestone indefinitely. Legacy-title recovery rules require current article fixtures before adaptation. No source is enabled automatically and subscriptions never start collection.

## Resulting coverage and verification

- **32 cataloged, 10 supported, 22 unsupported.** Supported: main academic; five Computing boards; four current Engineering boards. Unsupported: Engineering newsletter (1), Electronic (6), Mechanical (7), System Semiconductor (3), Semiconductor (5). Their full-article/resource contracts are not yet integrated and verified; old listing coverage is insufficient.
- Equivalent official Computing fixtures expose the public/internal-ID defect; Engineering fixtures cover all four boards plus a second historical list page, full article/table/image/attachment content and UTC/KST boundary cases. Reference tests were not rerun, so no cross-project pass-rate comparison is claimed.
- Production Rust bounded live test: one article per supported source, **10 articles, 17 resources, 7,294,862 local bytes, zero missing resources**. Offline Store reopening checks text/search/state and resource hashes. Every source in this sample is deliberately limited, not history-complete. Counts are observations, not permanent expectations.
- Isolated native macOS Engineering academic import: cancelled after two records, resumed to the observed five-record public endpoint; six resources downloaded and one unsupported legacy resource remained visible. This establishes only that small board's observed traversal, not comprehensive history across the catalog.
- Real native offline reading/search/saved access used network-denied sandbox processes. A deliberately corrupted **temporary-copy** image showed a placeholder/failure, offline retry reported failure, and explicit online repair restored the image. Existing attachment-role inline images also rendered without rewriting released data.

See [ROADMAP.md](ROADMAP.md) for release evidence and remaining platform/security gates.
