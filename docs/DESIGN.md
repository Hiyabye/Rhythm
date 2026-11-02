---
name: Rhythm
description: Korean-first, local-first macOS command center for Sogang notices, drawn in native Mac system grammar.
colors:
  bg: "#ffffff"
  sidebar: "#f0f0f2"
  surface: "#ffffff"
  pane: "#fafafb"
  field: "#ffffff"
  text: "#1d1d1f"
  text-2: "#3a3a3e"
  secondary: "#5c5c61"
  muted: "#68686d"
  line: "#e4e4e7"
  line-strong: "#cfcfd4"
  hover: "rgb(0 0 0 / .045)"
  accent: "#0a62d0"
  accent-fill: "#0a62d0"
  accent-text: "#ffffff"
  soft: "#e3edfb"
  danger: "#c0281c"
  family-main: "#d0344a"
  family-computing: "#d9730d"
  family-engineering: "#1c9a5f"
  family-other: "#8e8e93"
  bg-dark: "#1e1e20"
  sidebar-dark: "#29292c"
  surface-dark: "#232326"
  pane-dark: "#212124"
  field-dark: "#2a2a2d"
  text-dark: "#f2f2f5"
  text-2-dark: "#d6d6db"
  secondary-dark: "#afafb5"
  muted-dark: "#9c9ca3"
  line-dark: "#36363a"
  line-strong-dark: "#4a4a4f"
  hover-dark: "rgb(255 255 255 / .06)"
  accent-dark: "#6aa6ff"
  accent-fill-dark: "#2368d4"
  accent-text-dark: "#ffffff"
  soft-dark: "#233753"
  danger-dark: "#ff7c70"
  family-main-dark: "#ff6b7d"
  family-computing-dark: "#ffa24d"
  family-engineering-dark: "#3fcf8a"
  family-other-dark: "#98989f"
typography:
  display:
    fontFamily: "Pretendard Variable, Pretendard, -apple-system, BlinkMacSystemFont, sans-serif"
    fontSize: "26px"
    fontWeight: 700
    lineHeight: 1.25
    letterSpacing: "-0.02em"
  headline:
    fontFamily: "Pretendard Variable, Pretendard, -apple-system, BlinkMacSystemFont, sans-serif"
    fontSize: "15px"
    fontWeight: 650
  title:
    fontFamily: "Pretendard Variable, Pretendard, -apple-system, BlinkMacSystemFont, sans-serif"
    fontSize: "14px"
    fontWeight: 500
    lineHeight: 1.45
  body:
    fontFamily: "Pretendard Variable, Pretendard, -apple-system, BlinkMacSystemFont, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.5
  meta:
    fontFamily: "Pretendard Variable, Pretendard, -apple-system, BlinkMacSystemFont, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    fontFeature: "tnum"
  label:
    fontFamily: "Pretendard Variable, Pretendard, -apple-system, BlinkMacSystemFont, sans-serif"
    fontSize: "11px"
    fontWeight: 600
rounded:
  bar: "2px"
  row: "6px"
  panel: "8px"
  card: "10px"
  circle: "50%"
spacing:
  sidebar: "232px"
  sidebar-compact: "200px"
  sidebar-icon: "60px"
  week-pane: "340px"
  content-x: "32px"
  block-gap: "28px"
components:
  button-primary:
    backgroundColor: "{colors.accent-fill}"
    textColor: "{colors.accent-text}"
    rounded: "{rounded.row}"
    padding: "5px 14px"
    height: "28px"
  button-secondary:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.row}"
    padding: "5px 14px"
    height: "28px"
  source-item:
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.row}"
    padding: "5px 10px"
    height: "30px"
  source-item-active:
    backgroundColor: "{colors.soft}"
    textColor: "{colors.text}"
    rounded: "{rounded.row}"
  chip:
    textColor: "{colors.secondary}"
    typography: "{typography.meta}"
    rounded: "{rounded.row}"
    padding: "0 9px"
    height: "26px"
  chip-pressed:
    backgroundColor: "{colors.soft}"
    textColor: "{colors.accent}"
    rounded: "{rounded.row}"
  day-number-today:
    backgroundColor: "{colors.accent-fill}"
    textColor: "{colors.accent-text}"
    rounded: "{rounded.circle}"
    size: "26px"
  banner:
    backgroundColor: "{colors.pane}"
    textColor: "{colors.text}"
    rounded: "{rounded.panel}"
    padding: "11px 14px"
  week-pane:
    backgroundColor: "{colors.pane}"
    width: "340px"
    padding: "30px 20px 24px"
---

# Design — Quiet Intelligence

Rhythm is an information-first personal command center, not a chatbot dashboard. v0.3 made local analysis an optional layer beside the archive; v0.3.1 refined processing reliability. v0.4 adds user-controlled local profile, reversible feedback and explainable deterministic recommendations without making a model or internet access a prerequisite. v0.5 adds opt-in app lifecycle controls, scheduled incremental collection and native notification requests. v0.9 adds user-directed single-file backup/restore. v1.0.0 is the first stable release: Settings shows manual update steps (no automatic update control), an empty archive shows a short first-run guide on Today, and destructive actions (restore, personalization reset, deleting an interest or learned memory, clearing notification history, deleting a managed model) require a confirmation that names what changes and what is kept. Backup is not encrypted and excludes model weights; restore requires confirmation and a manual relaunch, and managed models survive only when local weights match expected size/SHA-256. Rhythm supports and targets macOS Apple Silicon only; previous Windows/Linux implementations and observations are historical, not current support. Release builds are ad-hoc signed and not notarized by design; Gatekeeper rejects them and README documents the per-app approval. Empty states distinguish cold start from tailored results; fictional summaries and fabricated activity do not fill them. Native behavior still requires the specifically described verification; do not infer distribution readiness from implementation.

The visual system is a native macOS app drawn without irony, with Things 3 and Fantastical as the craft bar (standing user decision recorded in `PRODUCT.md`). The YAML frontmatter above is the normative token list; `.impeccable/design.json` carries the matching component snippets, motion and breakpoints. The shell (sidebar, page header, banners) and Today were redesigned in this system. Settings, the reader and the AI surfaces already use the same tokens but keep their earlier geometry and have not been redesigned yet.

## Korean-first navigation

Navigation is a macOS source list. The four primary views **오늘 · 받은 소식 · 탐색 · 보관함** sit at the top, extension groups follow, and **설정** sits at the bottom. Labels, feedback and application-facing errors use natural Korean; product identifiers and model names stay recognizable.

| View | Current behavior |
| --- | --- |
| 오늘 | Recent ranked notices with grounded reasons; cold-start recent items are explicitly not personalized. Stored current AI summaries are optional and do not influence ranking. |
| 받은 소식 | Full paginated chronological archive with unread/source/date/original-category filters and save controls. Recommendations never silently replace the inbox. |
| 탐색 | Explicit local title/body search, mixed Korean/English/numeric terms, plain-text snippets and highlighted matches. |
| 보관함 | User-saved notices, independent of extension enablement. Saving contributes only bounded local evidence and does not make attachments available automatically. |
| 설정 | Profile, explicit interests and priority, learned memory/evidence, learning control/reset, source controls, model catalog and AI processing preferences; v0.5 adds lifecycle, scheduler, notification policy and history controls. |

Sidebar structure, top to bottom:

- **Brand row:** the Beat mark and the word "Rhythm".
- **Primary items:** each with a 17 px line icon in the accent colour. 받은 소식 shows the unread count and 보관함 the saved count as a plain tabular-numeral badge in secondary text, with a screen-reader description ("읽지 않은 공지 N개", "저장한 공지 N개"). A zero count is not shown.
- **Extension groups:** one group per extension, headed by its display name in a small label (11 px/600, secondary). For 서강대학교 공지, the supported subscribed boards are grouped under organization sub-labels (the "서강대학교 ·" prefix is removed), because board names repeat across institutions. Each board row has a small family-colour swatch next to its name. A disabled extension shows one quiet row, "꺼져 있음 · 설정에서 켜기"; an enabled extension with no subscribed boards shows "구독한 게시판 없음 · 설정". Both rows lead to Settings.
- **Board scope:** selecting a board opens 받은 소식 filtered to that board. The page title becomes the board name with "<organization> · 받은 소식" below it, and the board row, not 받은 소식, is marked current.
- **Bottom:** a sync status line, then 설정. While a sync runs, the line shows its message and page with a spinning refresh icon; otherwise it shows "마지막 확인 오늘 HH:MM" or "마지막 확인 M월 D일" from the latest successful board check, or "아직 가져온 적 없음" when subscribed boards were never fetched.

The page header is the view title (26 px) alone. On Today a line below it gives the date, the unread count and, after a sync with new notices, "최근 동기화로 새 공지 N". There is no eyebrow, caption, header note or footer.

`App.svelte` centralizes navigation, clears stale hash/scroll position and focuses main content. `ArchiveView.svelte` resets filters when changing sections, tracks asynchronous search generations and returns visible focus to the opened row on reader close, or to the list heading if that row disappeared. Saved/unread filters clamp to the last valid page after state changes, including late automatic read writes. Browser previews clearly identify desktop-only behavior and disable local operations rather than showing simulated persistence.


## Personalized Today and memory

Today requests a bounded set of recommendations from Rust/SQLite. Recommendations never require an analysis and opening a notice does not start inference.

Presentation. The centre column holds **나를 위한 소식** as inset list rows separated by hairlines. Each row shows the unread dot, the title (heavier when unread), a meta line with the board's family swatch, source, publication date, original category and "보관함에 저장됨" when saved, the deterministic reasons that contributed to its score, and an optional valid current AI summary labeled "저장된 AI 요약". The feedback chips **관심 있어요 / 관심 없어요** sit at the row's trailing edge. They appear when the row is hovered or contains focus, stay visible when pressed, and are always visible at 640 px and narrower, where they move below the row content. The same controls appear in the reader and can be reversed in either place.

A trailing 340 px **이번 주** pane on the right edge of the window lists Monday to Sunday with today's date number circled in the accent fill. Past days are dimmed. Each day lists notices published that day as events: a 3 px family-colour bar plus the title and a source meta line, so the colour is never the only cue. At most five notices appear per day, followed by "외 N건 · 받은 소식에서 모두 보기". AI-extracted reference dates appear in the matching day with a dotted grey bar and an "AI 참고 일정" label; dates after Sunday appear under **이후 AI 참고 일정** (at most five, labeled "AI · M월 D일"). Only dates from analyses that match the current contract are shown. A footnote says the reference dates come from the local AI, are not used for ranking or notifications, and should be checked in the original. When the week may extend beyond the loaded page, a footnote says only the newest notices are shown.

Below the picks, **최근 분석한 소식** lists at most five stored analyses: the short summary first, then "AI 분류 <category>", partial-analysis and contract-mismatch notes, up to two extracted dates, and the model name with a prompt to check the original.

Below 1100 px the week pane stops being a separate pane, and the order becomes picks → week → recent analyses in one column.

With no profile or interactions, Today labels the list as recent notices before personalization and links directly to interest settings. Explicit interest matches, explicit negative topics, profile matches, current feedback, saves, source subscription, freshness and meaningful revision recency affect ranking. Historical initial backfill receives no freshness reward; AI-generated dates and classifications do not rank notices. A modest two-pass source cap reduces same-source repetition without changing scores. The received-news list and search continue to expose the full archive.

Profile entries and explicit topics are clearly separated from learned topics. Only repeated positive feedback on or saving of different notices can create a learned topic; opening, reading and searching do not. The bounded topic extractor uses titles and original categories, not full bodies or AI output, and rejects a short list of obvious Korean/English sensitive terms; this lexical protection is not semantic classification. A document-level negative mark applies only to that notice. Settings displays the evidence count/action, allows editing a learned topic as explicit, deleting it, disabling learning or resetting personalization. Evidence expires after 180 days; deleting a topic records a sequence baseline so unchanged older activity alone cannot recreate it. These operations do not change subscriptions, saved state or archived content.

All profile, preference, feedback and evidence data remain in local SQLite. The personalization path has no LLM call, embedding service, remote ranking, notification or background work. Ranking reasons are emitted alongside the exact scoring signals, not generated from free-form model output.

## Archive list and search

Notices use restrained list rows rather than a dashboard of colored cards. Titles lead, followed by source, publication date, category and a **과거 자료** label where applicable. Unread state uses both a dot and accessible text; the save and read buttons name the action they perform ("저장 해제", "읽지 않음으로 표시") and expose no pressed state that could contradict that label. Constant-label toggles (interest feedback, theme, energy profile) use `aria-pressed`. Missing resources receive a visible note without implying the text is unavailable.

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

The bounded live/offline run exercised main-board entry `551681`, whose official body contained only an iframe. Current handling keeps the notice metadata and original fetched representation, leaves sanitized body fields empty, and neither fetches nor renders the iframe or its content. The reader explains that the body is unsupported and offers to open the original in the browser. Such an observation does not replace an existing readable revision; repeated metadata-only observations are allowed. This evidence is limited to the observed entry and does not establish general source reliability.

Disabling requests cancellation and waits for in-flight manual work, hides source controls and leaves stored text, resources, search and saved items accessible. Scheduler work is limited to enabled supported subscriptions and incremental collection, and stops when its extension/source eligibility is disabled. App-close-to-background is opt-in and requires a working tray; login launch and hidden login launch are opt-in. The scheduler is an in-process timer, not an OS service or wake alarm: no collection or notification processing occurs while the computer sleeps or after app exit. Actual sleep/wake catch-up behavior is not verified. AI analysis remains available for downloaded records when the extension is disabled.

## Lifecycle, scheduled synchronization and notifications

Settings groups lifecycle/scheduler controls under **백그라운드** and event policy/history under **알림**. All lifecycle/scheduler/notification defaults are off. Close-to-background hides only when tray setup succeeded; otherwise closing quits. Login registration is changed only by explicit user action and checked against OS state. Hidden startup affects the OS login launch, not ordinary app launch.

The scheduler wakes every 15 seconds while the process lives. Source tasks follow automatic interval `0` (the default), which uses Eco/Balanced/Performance at 60/30/15 minutes, or an explicit 15-minute–12-hour interval that always remains exact, including 60 minutes. It runs incremental sync only, not historical backfill. It can be paused, observes extension/subscription changes, and records next-run/last-outcome/errors. Its timer is not guaranteed to fire while the operating system is asleep and does not wake the machine; sleep/wake and catch-up behavior are unverified.

Notification sensitivity modes are **important** (score at least 18 and user preferences exist), **interested** (explicit interest match), and **all**; negative document feedback and negative-interest matches always exclude. Quiet hours are optional local wall time, default values 22:00–07:00 while disabled. Multiple events may appear as one digest. Event history is local and contains links to archived revisions, statuses and policy/error reasons, not copied bodies. OS-facing notification text includes source name and notice title.

Permission status and permission requests are separate from the in-app switch. When the native status/request API is unavailable, Settings says so and disables the request rather than implying authorization or delivery. Notification history labels a handoff as unconfirmed and activation as opened; dismissals and expiry are not represented as clicks.

The macOS authorization UI/status path was exercised in an ad-hoc-signed isolated debug app: the prompt was granted and status subsequently read as authorized. Current notifications use only the macOS-native `mac-usernotifications` 0.3.1 path; the earlier Windows/Linux Tauri notification-plugin path was removed during the macOS-only cutover. In an isolated synthetic test, the user saw and clicked a notice, Accessibility Inspector showed a matching archived title/body, and SQLite recorded one delivered event with attempt 1. After restart, its one dedupe row remained and no duplicate was observed. A separate quiet-hours test deferred three synthetic events with zero attempts; after simulated quiet-hour end, the user saw and clicked the grouped alert, Accessibility Inspector showed its history, and SQLite recorded two included events delivered while the third stayed deferred due to the rate limit. These observations establish only those isolated synthetic interactions, not live-source delivery or repeatable production behavior. A bounded hidden-window native scheduler test archived one article from each of two supported subscribed sources, seconds apart while a synthetic alert awaited response; a temporary debug-only cap of one page and one detail per source caused both tasks to record failure and left history incomplete. This demonstrates bounded execution and partial-archive/failure semantics, not a successful full sync. The temporary cap was removed; production has no debug testing override. A separate offline scheduler attempt persisted a network failure and 15-minute retry backoff, but not successful scheduled collection. Disabling and re-enabling the extension with the app stopped between retained both subscriptions and all 13 stored documents; SQLite integrity was `ok`. Login registration was simulated under a separate temporary HOME: a login item plist was created and removed, and `--autostart` with hidden-start enabled produced zero windows. This is not verification of an actual user login. These are historical, scoped macOS observations only; AI remains local, does not run to prepare notifications, and sends no analysis input/output or personalization evidence to a remote AI service.

## Local intelligence surfaces

First-run setup is a nonblocking app-shell banner with **로컬 AI 설정하기 / 나중에**. It never hides the archive or starts a weight download by itself. Settings shows OS/architecture/physical RAM, refreshed available disk capacity, catalog provenance/license/size, installation progress, cancellation/retry, selected versus loaded model and recovery feedback. Physical RAM is not presented as currently available memory or a promise that loading will succeed. Compatibility labels distinguish qualified verification, experimental and unsupported artifacts. Exact Qwen3.5 2B/4B entries have macOS Apple Silicon/Metal evidence; 9B remains experimental and cannot be installed with one click. These labels do not verify other backends/platforms or broad factual quality.

Model selection does not imply loaded memory. **선택한 모델 불러오기**, **모델 메모리 해제** and engine restart are separate actions. External GGUF registration explicitly references a full path without copying; reference removal never deletes external bytes. Managed deletion requires a second confirmation and says that documents/analyses remain. Setup completion performs a real short inference rather than treating download success as health. Backend-owned load/setup/register state shows the current phase and elapsed time, survives leaving and returning to Settings and offers cancellation. Cancellation may need to finish cooperative file verification or reap the worker before another operation starts; the UI does not announce success merely because a cancellation request was sent. Cancelled or failed setup does not persist completion.

The revision-keyed `AnalysisPanel.svelte` sits beside the reader header/body without replacing original content. It distinguishes intentional empty/no-model, pending/running, failure/retry, cached success, older model/prompt/schema/configuration and partial input. Revision-local job queries retain active work independently of bounded terminal history. Running analysis shows a backend phase and elapsed time, not a fabricated percentage; transient read failures offer retry and clear after successful refresh. Analyze/regenerate/cancel controls do not disable reading or saving. Existing results are accessible without the generating model. Old successful output remains visible when regeneration fails, and history retains model provenance.

Short summary and central AI category lead; detailed summary, extracted facts/dates and expandable **원문 근거** follow. AI classification is labeled separately from the original board category. Extracted dates are informational, not official calendar events. Partial results show actual selected/total character counts and warn that omitted intervals may contain important information. Long input selects bounded exact head/tail/important passages, deduplicating exact repeated text only when over budget and keeping structural table rows as separate evidence units. This is not complete-document understanding or chunk-and-merge. Generated fields render as escaped text, not HTML; evidence validation is not a promise of semantic accuracy.

Today’s recommendation list is independent of AI analysis. A separate section shows at most five real stored analyses, with their contract age labeled; the list remains usable if analysis lookup fails or no model is installed. Other archive rows use compact optional analysis indicators without requiring every item to be processed; search continues showing original matching snippets. There is no conversational transcript, glowing AI decoration, personalized “you must do this” claim, automatic calendar action or notification.

**AI 처리** defaults to explicit-only. The optional recent mode and per-open-session cap remain small. Candidates follow latest-revision collection time rather than publication date: new nonhistorical arrivals and meaningful revisions, including updates to historically collected documents, may qualify; initial backfill, startup history and previously attempted revisions do not. Advanced controls expose output length rather than dozens of sampling parameters. A queue lists status, failure and cancellation. Model/backend/native-quality verification is separate from UI implementation; see [AI evaluation](AI_EVALUATION.md).

## Typography and layout

Pretendard Variable is imported locally through the `pretendard` npm package in `src/main.ts`; startup does not use a font CDN. The font stack falls back to native system fonts and sans-serif. Its SIL Open Font License is retained at `assets/licenses/Pretendard-OFL.txt` and bundled as an application resource.

Type sits at macOS system sizes on a 13 px base (line height 1.5). Hierarchy comes from size and weight, not colour or decoration:

- **Page title** (700, 26 px, line height 1.25, −0.02em): one per view, in the page header.
- **Pane heading** (650, 15 px): 나를 위한 소식, 최근 분석한 소식, 이번 주, with a 12 px secondary note on the same baseline.
- **Row title** (500, 14 px, line height 1.45; 650 when unread): pick titles and recent-analysis summaries.
- **Body** (400, 13 px): sidebar items, banners, buttons.
- **Meta** (400, 12 px, tabular numerals): source/date lines, reasons (in `--text-2`), chips, counts.
- **Label** (600, 11 px): sidebar group headings, weekday names, event meta and footnotes (400).
- **Day number** (600, 15 px, tabular numerals) in a 26 px circle.

**The Tabular Count Rule.** Every count, date and time (sidebar badges, the Today subline, sync time, day numbers, meta lines) uses tabular numerals so columns and badges don't shift.

**The No-Kicker Rule.** No eyebrows, letter-spaced kickers, header notes or taglines above or around titles. The page title stands alone; context goes in the line below it.

Layout is three panes on Today and two elsewhere. The sidebar is fixed at 232 px (200 px below 1100 px; a 60 px icon column below 800 px). Main content is padded 26 px top, 32 px sides and 40 px bottom (26 px sides below 1100 px; 22/20/32 px below 800 px). On Today a fixed 340 px week pane occupies the right edge and main content reserves that width. Today's blocks are separated by 28 px. Below 1100 px the week pane joins the main column (picks → week → recent analyses); at 640 px and narrower the feedback chips move below each pick. Settings keeps its internal section list and restrained content column; archive lists and reader share the same surfaces. Native window controls and a minimum window size are retained. The reader region is keyboard-focusable, has a Korean label/visible outline and scrolls wide content with horizontal arrows. Narrow native macOS reading and browser WebKit/Chromium 640 px/200% wide-table containment were checked. Browser results and historical Windows/Linux builds do not establish support for other native platforms.

Depth is flat. Panes are separated by 1 px hairlines (`--line`) and the tonal step between `--bg`, `--sidebar` and `--pane`, not by shadows. The only shadows are small control affordances: the selected segment (`0 1px 3px #0000000a`), the switch knob (`0 1px 2px #0002`) and the transient save notice (`0 4px 24px #0000000c`). Corners are 6 px for rows, buttons, chips and events, 8 px for pick rows and banners, 10 px for AI panels, and 2 px for swatches and event bars. Motion is native-quick: hover, selection and chip-reveal fades of 140–150 ms, a spinning icon only while a sync runs, and everything disabled under `prefers-reduced-motion`.

The sidebar is opaque `--sidebar`, not macOS vibrancy; translucency would need a Tauri window setting and is an accepted gap.

Shared styles live in `src/app.css`, with AI surfaces in `src/lib/ai.css`; reusable symbols are in `src/lib/Icon.svelte`. Tailwind CSS is integrated, while semantic component classes define the visual system.

## Color

One system-blue accent on neutral greys, with four calendar-style family colours for institutions. Token values are in the frontmatter (light keys, and `-dark` keys for `:root[data-theme=dark]`).

- **Neutrals.** `--bg` and `--surface` are white in light mode and graphite (`#1e1e20` / `#232326`) in dark mode. `--sidebar` is a slightly darker grey than the content. `--pane` is a barely tinted surface for the week pane and banners. `--field` backs inputs.
- **Text.** `--text` for titles and body, `--text-2` for secondary content that still needs weight (recommendation reasons, chip text on narrow screens, the brand name), `--secondary` for meta lines and notes, `--muted` for placeholders, empty days and footnotes.
- **Lines.** `--line` is the hairline for separators and pane edges. `--line-strong` is for control outlines that must read as edges (secondary buttons, the "AI 참고 일정" label).
- **Accent.** `--accent` is the accent as text or icon: sidebar icons, text buttons, the unread dot, focus rings, today's weekday name, pressed chips. `--accent-fill` is the accent as a filled background behind `--accent-text`: primary buttons and today's date circle. They are the same in light mode; in dark mode the fill is deeper (`#2368d4`) so white text on it stays legible, while the text accent is lighter (`#6aa6ff`). `--soft` is the selection tint for the active sidebar row, pressed chips and text selection.
- **Family colours.** `--family-main` (main academic board), `--family-computing`, `--family-engineering` and `--family-other` (fallback). They appear only as 8 px swatches in the sidebar and pick meta lines and as 3 px event bars in the week pane. AI reference dates use a dotted `--muted` bar instead.
- **Danger.** `--danger` for error banners (text and border, transparent background) and destructive text.

**The One Accent Rule.** Blue is the only chromatic UI colour. It marks selection, focus, the unread dot, today's date and primary actions, nothing else.

**The Colour-Plus-Text Rule.** A family colour always sits next to a text label (board name or source name). Colour identifies an institution at a glance; the text carries the meaning.

**The Fill-Versus-Text Rule.** White text goes on `--accent-fill`, never on `--accent`. Use `--accent` only for text and icons on neutral backgrounds.

Contrast, computed from the token values (WCAG relative luminance), not measured on screen:

- `--text`, `--text-2` and `--secondary` exceed 4.5:1 on `--bg`, `--sidebar`, `--pane` and `--soft` in both themes (lowest: `--secondary` on `--soft`, 5.63 light, 5.52 dark).
- `--muted` exceeds 4.5:1 on `--bg`, `--sidebar` and `--pane` in both themes (lowest 4.87 light on `--sidebar`).
- `--accent` is 5.72 on `--bg` light and 5.59 dark; white on `--accent-fill` is 5.72 light and 5.25 dark.
- `--accent` on `--soft` is 4.84 light and 4.88 dark, so pressed chip text (12 px/600) stays above 4.5:1 in both themes.
- Family colours as non-text marks: dark-theme bars and swatches are 5.0:1 or more on `--pane` and `--sidebar`. Light-theme bars on `--pane` are 3.13–4.71; light-theme swatches on `--sidebar` are 2.86 (`--family-other`) and 2.88 (`--family-computing`), below 3:1. The swatch is decorative and always paired with the board name.

### Visual guardrails

- **Do** use `--accent-fill` behind white text and `--accent` for accent text and icons.
- **Do** pair every family colour with its board or source name.
- **Do** use tabular numerals for counts, dates and times.
- **Do** keep feedback chips hidden until row hover/focus, visible when pressed, and always visible at 640 px and narrower.
- **Don't** add eyebrows, letter-spaced kickers, header notes, taglines or stat tiles.
- **Don't** introduce a second chromatic accent beyond the family marks.
- **Don't** separate panes with shadows; use hairlines and tonal steps.

State is not communicated through color alone. Thin separators organize content without purple gradients, glowing AI ornaments or fabricated activity indicators.

## Interaction and accessibility boundaries

Theme selection applies immediately, persists through Rust and reverts on save failure; system mode follows OS preference. Energy profiles control app-open analysis budgets and idle model release, and also select scheduled-sync cadence when interval is automatic (`0`, the default). Eco starts no automatic AI analysis; Balanced and Performance allow bounded, opt-in recent work while the window is focused. Scheduled archive sync is independently opt-in; explicit intervals, including 60 minutes, remain exact. Effects are short and reduced-motion styles are included.

The interface includes labels, icon-button names, native keyboard-operable controls, focus-visible outlines, a skip link and status/alert semantics. These are implementation properties, **not full accessibility conformance**. Historical v0.2.1 browser/native checks and v0.3.1 mocked-IPC/native checks do not verify v0.5 lifecycle and notification UI or current native behavior. Recent automated checks use mocked IPC; the bounded native observations above cover only their explicitly described interactions. Full VoiceOver, notification display/action behavior, macOS installation and recovery remain unverified.

v0.9.2 native review (isolated validation bundle, macOS 27.2, Accessibility API inspection, not spoken VoiceOver output): the WebKit tree exposes the skip link, complementary/navigation/main landmarks, one `h1` per page with ordered `h2`/`h3`, named pop-ups/text fields/disclosures, switch and toggle states, disabled dependents, status/alert roles for progress and errors, and the native confirmation sheet's title, message and buttons. Tab order follows the visual order, skips disabled controls, and cycles back to the skip link without a trap. Opening a notice with Return moves focus to its heading; Escape returns focus to the same row. The focus ring is visible in dark theme, and the icon-only sidebar at the 640 px minimum width keeps its names. Theme token pairs meet 4.5:1 except `--muted` on `--soft` (4.41 light, 3.88 dark), a pair no current element renders. Fixed defects: action-named read/save buttons also exposed a contradictory pressed state, the sidebar announced a stale version, and static manual-update text was a live region. Not reviewed: spoken VoiceOver output and rotor, text scaling/zoom, increased contrast and reduced motion at the OS level. Historical Windows/Linux webview behavior is not current support; see [the roadmap](ROADMAP.md).

v1.0.0 review (macOS 27.2, the release app copied from its DMG, Accessibility API inspection and AX actions, not spoken VoiceOver output). Fixed: the settings side navigation had silently lost links to General, Extensions, AI, Personalization and Background in v0.9.0 and now links every section; the learning switch announced its state but never showed it visually; repeated per-item buttons (interest edit/delete, notification history open) now include the item name; stored scheduler errors and history errors are plain text instead of alerts re-announced on every visit; the General "시작과 종료" row had no control and now links to Background. The first-run guide is a named region on an empty Today. Native confirmation sheets were observed for interest deletion, and cancelling left the interest in place. Still not reviewed: spoken VoiceOver output and rotor, OS text size and zoom, increased contrast, and reduced motion at the OS level (reduced-motion CSS exists but was not observed natively). No `prefers-contrast` styling exists.

The native-macOS redesign of the shell (sidebar, page header, banners) and Today, after v1.0.0, was checked only with mocked-IPC Playwright captures in Chromium and WebKit (light, dark, empty Today, 1000 px and 640 px widths, feedback hover/focus, a board-scoped inbox and Settings). It has not been natively reviewed: none of the v0.9.2 or v1.0.0 observations above apply to the new sidebar, week pane or feedback chips. The token contrast values in [Color](#color) are computed, not measured. The sidebar with all ten supported boards subscribed has not been captured. The sidebar is opaque rather than vibrant, and there is still no `prefers-contrast` styling.

## Beat branding

`assets/branding/beat.svg` is the original vector source. Four rounded bars of different heights form a rhythm recognizable at small sizes. Generated desktop icon formats in `src-tauri/icons/` remain committed. The redesign did not touch the source SVG or the desktop icon, which still use the earlier pre-redesign colours (`#F4F1EA` bars on `#344C43`); they are not part of the current palette and do not match the in-app accent.

In the app, the mark is a small 16 px-high set of four 3 px bars in `--accent`, next to the word "Rhythm" in the sidebar brand row. It follows the theme through the accent token. Branding can evolve without changing application boundaries. The original logo follows Rhythm's MIT license; font/dependency licenses and the rights to school content remain separate.
