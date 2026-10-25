# Design — Quiet Intelligence

Rhythm is a personal command center, not a chatbot-first dashboard. v0.2 makes actual local notices useful without AI, a model or an internet connection; v0.2.1 strengthens failure recovery, keyboard use and readability without redesigning the interface. Empty space communicates missing information; fictional notices and fabricated activity do not fill it.

## Korean-first navigation

Main navigation remains **오늘 · 받은 소식 · 탐색 · 보관함 · 설정**. Labels, feedback and application-facing errors use natural Korean; product identifiers and model names stay recognizable.

| View | v0.2 behavior |
| --- | --- |
| 오늘 | Actual archived/unread/saved/new-notice counts and recent local notices. Historical imports are excluded from the new-notice count. No generated briefing or relevance ranking. |
| 받은 소식 | Paginated local notices with unread indicators, source/date/category metadata, filters and save controls. |
| 탐색 | Explicit local title/body search, mixed Korean/English/numeric terms, plain-text snippets and highlighted matches. |
| 보관함 | User-saved notices, independent of extension enablement. Saving does not mean every attachment was downloaded. |
| 설정 | Existing preferences plus extension opt-in, grouped source subscriptions, manual collection and progress/error controls. |

`App.svelte` centralizes navigation, clears stale hash/scroll position and focuses main content. `ArchiveView.svelte` resets filters when changing sections, tracks asynchronous search generations and returns visible focus to the opened row on reader close, or to the list heading if that row disappeared. Saved/unread filters clamp to the last valid page after state changes, including late automatic read writes. Browser previews clearly identify desktop-only behavior and disable local operations rather than showing simulated persistence.

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

Disabling requests cancellation and waits for in-flight work, hides source controls and leaves stored text, resources, search and saved items accessible. The UI does not present notifications, AI summaries, automatic scheduling or energy-aware crawling as operational.

## Typography and layout

Pretendard Variable is imported locally through the `pretendard` npm package in `src/main.ts`; startup does not use a font CDN. The font stack falls back to native system fonts and sans-serif. Its SIL Open Font License is retained at `assets/licenses/Pretendard-OFL.txt` and bundled as an application resource.

Hierarchy comes from type size/weight, whitespace and alignment. A stable left sidebar anchors navigation. Settings uses an internal section list and restrained content column; archive lists and reader share the same surfaces. At narrower widths sidebar labels yield to icon navigation and controls reflow. Native window controls and a minimum window size are retained. The reader region is keyboard-focusable, has a Korean label/visible outline and scrolls wide content with horizontal arrows. Narrow native macOS reading and browser WebKit/Chromium 640 px/200% wide-table containment were checked; full cross-platform native coverage remains open.

Styles live in `src/app.css`; reusable symbols are in `src/lib/Icon.svelte`. Tailwind CSS is integrated, while semantic component classes define the visual system.

## Color

| Role | Color |
| --- | --- |
| Forest green | `#344C43` |
| Warm ivory | `#F4F1EA` |
| Dark foundation | `#202622` |
| Muted green reference | `#A1AA9C` |

Light mode uses a near-white main surface (`#FAF9F6`) and warmer sidebar (`#F0EEE7`). Dark mode changes the same semantic tokens for surfaces, text, dividers, interaction and errors. Color emphasizes selection, focus and primary actions; state is not communicated through color alone. Light-theme secondary/muted semantic text was darkened to meet a tested 4.5:1 normal-text contrast minimum against main, surface and sidebar backgrounds. Thin separators organize content without purple gradients, glowing AI ornaments or fabricated activity indicators.

## Interaction and accessibility boundaries

Theme selection applies immediately, persists through Rust and reverts on save failure; system mode follows OS preference. Energy profiles remain stored preferences without scheduling effects. Effects are short and reduced-motion styles are included.

The interface includes labels, icon-button names, native keyboard-operable controls, focus-visible outlines, a skip link and status/alert semantics. These are implementation properties, **not full accessibility conformance**. v0.2.1 adds compiled-frontend Chromium/WebKit tests for focus/keyboard scroll, settings/themes, failure states, reader markup and paging. Isolated native macOS checks additionally exercised source controls, read/save/search persistence, network-denied local images, corruption/repair, folder reveal, focusable content and narrow/light/dark layout. Full VoiceOver navigation and Windows/Linux webviews remain verification gates in [the roadmap](ROADMAP.md). Mocked IPC tests do not replace real Rust/native tests.

## Beat branding

`assets/branding/beat.svg` is the original vector source. Four rounded bars of different heights form a rhythm recognizable at small sizes; the desktop icon uses ivory bars on forest green. Generated desktop icon formats in `src-tauri/icons/` remain committed.

The in-app mark uses `currentColor` for both themes. Branding can evolve without changing application boundaries. The original logo follows Rhythm's MIT license; font/dependency licenses and the rights to school content remain separate.
