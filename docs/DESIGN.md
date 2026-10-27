# Design — Quiet Intelligence

Rhythm is an information-first personal command center, not a chatbot dashboard. v0.3 made local analysis an optional layer beside the archive; v0.3.1 refined processing reliability. v0.4 adds user-controlled local profile, reversible feedback and explainable deterministic recommendations without making a model or internet access a prerequisite. Empty states distinguish cold start from tailored results; fictional summaries and fabricated activity do not fill them. The v0.2.1 recovery, keyboard and reader boundaries remain.

## Korean-first navigation

Main navigation remains **오늘 · 받은 소식 · 탐색 · 보관함 · 설정**. Labels, feedback and application-facing errors use natural Korean; product identifiers and model names stay recognizable.

| View | v0.4 behavior |
| --- | --- |
| 오늘 | Recent ranked notices with grounded reasons; cold-start recent items are explicitly not personalized. Stored current AI summaries are optional and do not influence ranking. |
| 받은 소식 | Full paginated chronological archive with unread/source/date/original-category filters and save controls. Recommendations never silently replace the inbox. |
| 탐색 | Explicit local title/body search, mixed Korean/English/numeric terms, plain-text snippets and highlighted matches. |
| 보관함 | User-saved notices, independent of extension enablement. Saving contributes only bounded local evidence and does not make attachments available automatically. |
| 설정 | Profile, explicit interests and priority, learned memory/evidence, learning control/reset, source controls, model catalog and AI processing preferences. |

`App.svelte` centralizes navigation, clears stale hash/scroll position and focuses main content. `ArchiveView.svelte` resets filters when changing sections, tracks asynchronous search generations and returns visible focus to the opened row on reader close, or to the list heading if that row disappeared. Saved/unread filters clamp to the last valid page after state changes, including late automatic read writes. Browser previews clearly identify desktop-only behavior and disable local operations rather than showing simulated persistence.


## Personalized Today and memory

Today requests a bounded set of recommendations from Rust/SQLite. The quiet list shows title, source, publication date, source category, read/saved state, an optional valid current AI summary, and only the deterministic reasons that contributed to its score. Interest feedback can be reversed on the row; the same controls appear in the reader. Recommendations never require an analysis and opening a notice does not start inference.

With no profile or interactions, Today labels the list as recent notices before personalization and links directly to interest settings. Explicit interest matches, explicit negative topics, profile matches, current feedback, saves, source subscription, freshness and meaningful revision recency affect ranking. Historical initial backfill receives no freshness reward; AI-generated dates and classifications do not rank notices. A modest two-pass source cap reduces same-source repetition without changing scores. The received-news list and search continue to expose the full archive.

Profile entries and explicit topics are clearly separated from learned topics. Only repeated positive feedback on or saving of different notices can create a learned topic; opening, reading and searching do not. The bounded topic extractor uses titles and original categories, not full bodies or AI output, and rejects a short list of obvious Korean/English sensitive terms; this lexical protection is not semantic classification. A document-level negative mark applies only to that notice. Settings displays the evidence count/action, allows editing a learned topic as explicit, deleting it, disabling learning or resetting personalization. Evidence expires after 180 days; deleting a topic records a sequence baseline so unchanged older activity alone cannot recreate it. These operations do not change subscriptions, saved state or archived content.

All profile, preference, feedback and evidence data remain in local SQLite. There is no LLM call, embedding service, remote ranking, notification or background work in this path. Ranking reasons are emitted alongside the exact scoring signals, not generated from free-form model output.

## Archive list and search

Notices use restrained list rows rather than a dashboard of colored cards. Titles lead, followed by source, publication date, category and a **과거 자료** label where applicable. Unread state uses both a dot and accessible text; saved state has an independent pressed button. Missing resources receive a visible note without implying the text is unavailable.

Source/category selectors and publication-date order apply to actual local records. Inbox adds an unread-only control. Search runs on explicit submission, not a remote suggestion service; its explanatory text says that whitespace-separated terms must all occur and queries stay local. Highlighting renders escaped text and `mark` elements, never arbitrary search-result HTML. Lists use 30-row pages, guard pending save writes and announce loading/result/empty status without dialogs. Removing the final item from a later page never implies the entire archive is empty.

Empty states distinguish: no archive yet, no saved notices, no matching filters and load/search failure. They offer source settings or a clear retry when appropriate. A loading indicator is not a success count; a failed response does not fabricate an empty completed import.

## Reader and resource recovery

`DocumentReader.svelte` presents source, title, date, author/category, read/saved actions and **학교 원문 열기** above the stored body. Opening a notice attempts to mark it read; the user can reverse that state. Retrieved content stays readable if this write fails, with non-blocking Korean feedback and no false read-state success. Earlier meaningful snapshots are selectable in **보관 이력**, while read/saved state remains attached to the document.

The reader is a restricted local document surface, not an embedded live school page. It preserves useful headings, paragraphs, emphasis, scientific subscripts/superscripts, lists and tables without school navigation, scripts, forms or styling. Original fetched JSON/HTML is stored for archival purposes but is never injected directly. Local images come only from integrity-checked raster data URIs; unavailable images become explanatory placeholders. There is no remote-image fallback. Repeated inline occurrences share one local image validation per render; obsolete renders stop reading subsequent resources. Released archives with an attachment-role inline raster remain supported through the same backend integrity/format boundary.

Body text and linked resources have separate status. The resource section shows original filename, downloaded/missing/failed/oversized/unsupported state, size, MIME and optional SHA-256. **저장 폴더에서 보기** reveals a verified local file without executing it. **다시 내려받기** is an explicit network action, disabled when the extension is off or another resource job is running.

A persisted `downloaded` label is not proof the file still exists. Observed local-read/reveal failures are tracked per resource, override the success label and remove the reader's “all stored” claim. Metadata refresh alone does not erase that error. Successful local validation can clear it; retry remains available even for a formerly downloaded file. Missing/corrupt bytes must not silently cause remote retrieval.

External links carry a visible external marker and open through the OS browser only after activation. HTTP(S) link metadata is not automatic-download permission. Esc returns to the list unless a select is being operated; native controls, heading focus and status/alert feedback support keyboard use.

## Source subscriptions and collection

The extension switch represents persisted opt-in, not a background scheduler. Enabling **서강대학교 공지** reveals `SourceSettings.svelte`; source subscriptions are separate switches grouped by institution/department with a text filter. The 32-source catalog clearly distinguishes ten supported sources from 22 **원문 수집 미지원** sources, including the four current Engineering boards only after full-article/local-resource verification. Unsupported switches are disabled, not optimistically labeled working.

Actions distinguish **과거 자료 가져오기 / 이어받기** from **새 소식 및 변경 확인**. Selection checkboxes target a subset of subscribed sources; no selection targets all supported subscriptions. Subscribing alone starts nothing. Progress distinguishes historical/incremental/resource work and shows current source/page/message, persisted attempt/success times, per-attempt discovered/archived/failed counts, cumulative checkpoint and error. Never-imported sources are not described as interrupted history; `limited` means partial work that can continue, not necessarily a university access restriction.

Historical collection can be cancelled and resumed. Limited, interrupted or failed work is not labeled complete. “Public historical pages complete” does not mean every attachment succeeded. The initial incremental seed is recent historical material, not a flood of new announcements. Incremental checks revisit recent/pinned records and discover newer records; they do not promise a full audit of every old edit.

Disabling requests cancellation and waits for in-flight work, hides source controls and leaves stored text, resources, search and saved items accessible. The UI does not present notifications, automatic collection/scheduling or energy-aware crawling as operational. AI analysis is independent of collection and remains available for downloaded records when the extension is disabled.

## Local intelligence surfaces

First-run setup is a nonblocking app-shell banner with **로컬 AI 설정하기 / 나중에**. It never hides the archive or starts a weight download by itself. Settings shows OS/architecture/physical RAM, refreshed available disk capacity, catalog provenance/license/size, installation progress, cancellation/retry, selected versus loaded model and recovery feedback. Physical RAM is not presented as currently available memory or a promise that loading will succeed. Compatibility labels distinguish qualified verification, experimental and unsupported artifacts. Exact Qwen3.5 2B/4B entries have macOS Apple Silicon/Metal evidence; 9B remains experimental and cannot be installed with one click. These labels do not verify other backends/platforms or broad factual quality.

Model selection does not imply loaded memory. **선택한 모델 불러오기**, **모델 메모리 해제** and engine restart are separate actions. External GGUF registration explicitly references a full path without copying; reference removal never deletes external bytes. Managed deletion requires a second confirmation and says that documents/analyses remain. Setup completion performs a real short inference rather than treating download success as health. Backend-owned load/setup/register state shows the current phase and elapsed time, survives leaving and returning to Settings and offers cancellation. Cancellation may need to finish cooperative file verification or reap the worker before another operation starts; the UI does not announce success merely because a cancellation request was sent. Cancelled or failed setup does not persist completion.

The revision-keyed `AnalysisPanel.svelte` sits beside the reader header/body without replacing original content. It distinguishes intentional empty/no-model, pending/running, failure/retry, cached success, older model/prompt/schema/configuration and partial input. Revision-local job queries retain active work independently of bounded terminal history. Running analysis shows a backend phase and elapsed time, not a fabricated percentage; transient read failures offer retry and clear after successful refresh. Analyze/regenerate/cancel controls do not disable reading or saving. Existing results are accessible without the generating model. Old successful output remains visible when regeneration fails, and history retains model provenance.

Short summary and central AI category lead; detailed summary, extracted facts/dates and expandable **원문 근거** follow. AI classification is labeled separately from the original board category. Extracted dates are informational, not official calendar events. Partial results show actual selected/total character counts and warn that omitted intervals may contain important information. Long input selects bounded exact head/tail/important passages, deduplicating exact repeated text only when over budget and keeping structural table rows as separate evidence units. This is not complete-document understanding or chunk-and-merge. Generated fields render as escaped text, not HTML; evidence validation is not a promise of semantic accuracy.

Today’s recommendation list is independent of AI analysis. A separate section shows at most five real stored analyses, with their contract age labeled; the list remains usable if analysis lookup fails or no model is installed. Other archive rows use compact optional analysis indicators without requiring every item to be processed; search continues showing original matching snippets. There is no conversational transcript, glowing AI decoration, personalized “you must do this” claim, automatic calendar action or notification.

**AI 처리** defaults to explicit-only. The optional recent mode and per-open-session cap remain small. Candidates follow latest-revision collection time rather than publication date: new nonhistorical arrivals and meaningful revisions, including updates to historically collected documents, may qualify; initial backfill, startup history and previously attempted revisions do not. Advanced controls expose output length rather than dozens of sampling parameters. A queue lists status, failure and cancellation. Model/backend/native-quality verification is separate from UI implementation; see [AI evaluation](AI_EVALUATION.md).

## Typography and layout

Pretendard Variable is imported locally through the `pretendard` npm package in `src/main.ts`; startup does not use a font CDN. The font stack falls back to native system fonts and sans-serif. Its SIL Open Font License is retained at `assets/licenses/Pretendard-OFL.txt` and bundled as an application resource.

Hierarchy comes from type size/weight, whitespace and alignment. A stable left sidebar anchors navigation. Settings uses an internal section list and restrained content column; archive lists and reader share the same surfaces. At narrower widths sidebar labels yield to icon navigation and controls reflow. Native window controls and a minimum window size are retained. The reader region is keyboard-focusable, has a Korean label/visible outline and scrolls wide content with horizontal arrows. Narrow native macOS reading and browser WebKit/Chromium 640 px/200% wide-table containment were checked; full cross-platform native coverage remains open.

Shared styles live in `src/app.css`, with AI surfaces in `src/lib/ai.css`; reusable symbols are in `src/lib/Icon.svelte`. Tailwind CSS is integrated, while semantic component classes define the visual system.

## Color

| Role | Color |
| --- | --- |
| Forest green | `#344C43` |
| Warm ivory | `#F4F1EA` |
| Dark foundation | `#202622` |
| Muted green reference | `#A1AA9C` |

Light mode uses a near-white main surface (`#FAF9F6`) and warmer sidebar (`#F0EEE7`). Dark mode changes the same semantic tokens for surfaces, text, dividers, interaction and errors. Color emphasizes selection, focus and primary actions; state is not communicated through color alone. Light-theme secondary/muted semantic text was darkened to meet a tested 4.5:1 normal-text contrast minimum against main, surface and sidebar backgrounds. Thin separators organize content without purple gradients, glowing AI ornaments or fabricated activity indicators.

## Interaction and accessibility boundaries

Theme selection applies immediately, persists through Rust and reverts on save failure; system mode follows OS preference. Energy profiles now control app-open analysis budgets and idle model release, not collection scheduling. Eco starts no automatic analysis; Balanced and Performance allow bounded, opt-in recent work while the window is focused. Effects are short and reduced-motion styles are included.

The interface includes labels, icon-button names, native keyboard-operable controls, focus-visible outlines, a skip link and status/alert semantics. These are implementation properties, **not full accessibility conformance**. v0.2.1 adds compiled-frontend Chromium/WebKit tests for focus/keyboard scroll, settings/themes, failure states, reader markup and paging. Isolated native macOS checks additionally exercised source controls, read/save/search persistence, network-denied local images, corruption/repair, folder reveal, focusable content and narrow/light/dark layout. Full VoiceOver navigation and Windows/Linux webviews remain verification gates in [the roadmap](ROADMAP.md). The new AI browser cases cover setup, model controls, analysis/error/cancel/cache/partial states, escaped generated text and revision races using mocked IPC. Historical v0.3.0 native checks additionally exercised 2B/Metal setup, analysis, offline regeneration and failure recovery. v0.3.1 has 86 mocked-IPC Chromium/WebKit checks and targeted isolated native load/remount/cancel, setup and network-denied prompt-v3 regeneration evidence. Bounded model comparisons completed and justified analysis-only constrained decoding for structural reliability; setup remains ordinary text. Structurally valid outputs still included missed/obsolete deadlines and reversed exclusions, so neither grammar nor prompt/representation changes guarantee semantic correctness. Full final-mode factual-quality evaluation and human review remain open. Browser offline mode does not prove native offline inference. Mocked IPC tests do not replace real Rust/native tests.

## Beat branding

`assets/branding/beat.svg` is the original vector source. Four rounded bars of different heights form a rhythm recognizable at small sizes; the desktop icon uses ivory bars on forest green. Generated desktop icon formats in `src-tauri/icons/` remain committed.

The in-app mark uses `currentColor` for both themes. Branding can evolve without changing application boundaries. The original logo follows Rhythm's MIT license; font/dependency licenses and the rights to school content remain separate.
