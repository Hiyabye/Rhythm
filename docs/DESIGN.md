---
name: Rhythm
description: Korean-first, local-first desktop command center for Sogang notices (macOS Apple Silicon and Windows 11), drawn in native Mac system grammar.
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
  pane-title:
    fontFamily: "Pretendard Variable, Pretendard, -apple-system, BlinkMacSystemFont, sans-serif"
    fontSize: "20px"
    fontWeight: 700
    lineHeight: 1.3
    letterSpacing: "-0.015em"
rounded:
  bar: "2px"
  row: "6px"
  panel: "8px"
  card: "10px"
  pill: "13px"
  circle: "50%"
spacing:
  sidebar: "232px"
  sidebar-compact: "200px"
  sidebar-icon: "60px"
  week-pane: "340px"
  content-x: "32px"
  block-gap: "28px"
  settings-nav: "176px"
  settings-pane: "680px"
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
  settings-group:
    backgroundColor: "{colors.pane}"
    rounded: "{rounded.card}"
  settings-row:
    textColor: "{colors.text}"
    typography: "{typography.body}"
    padding: "8px 14px"
    height: "46px"
  pane-item-active:
    backgroundColor: "{colors.soft}"
    textColor: "{colors.text}"
    rounded: "{rounded.row}"
    height: "30px"
  board-tile:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text-2}"
    rounded: "{rounded.card}"
    padding: "11px 12px"
    height: "72px"
  board-tile-on:
    backgroundColor: "{colors.accent-fill}"
    textColor: "{colors.accent-text}"
    rounded: "{rounded.card}"
  model-tile:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.card}"
    padding: "12px 12px 10px 14px"
  model-tile-selected:
    backgroundColor: "{colors.soft}"
    textColor: "{colors.text}"
    rounded: "{rounded.card}"
  interest-chip:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.pill}"
    padding: "2px 11px"
    height: "26px"
  popover:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text-2}"
    rounded: "{rounded.card}"
    padding: "12px 14px"
    width: "300px"
---

# Design — Quiet Intelligence

Rhythm is an information-first personal command center, not a chatbot dashboard. The archive is the core; local AI is an optional, labeled layer beside it, and recommendations come from deterministic local signals that work without a model or network. Background features are opt-in, except the announce-only automatic update check in builds with the updater key; builds without it show manual update steps. An empty archive shows a short first-run guide on Today. Destructive actions (restore, personalization reset, deleting an interest or learned memory, clearing notification history, deleting a managed model) require a confirmation that names what changes and what is kept. Empty states distinguish a cold start from tailored results; fictional summaries and fabricated activity never fill them. The same UI runs on macOS and Windows; platform wording follows the WebView user agent.

The visual system is a native macOS app drawn without irony, with Things 3 and Fantastical as the craft bar (standing user decision recorded in `PRODUCT.md`). The YAML frontmatter above is the normative token list; `.impeccable/design.json` carries the matching component snippets, motion and breakpoints. The shell (sidebar, page header, banners), Today and Settings were redesigned in this system. The reader and the reader's AI panel already use the same tokens but keep their earlier geometry and have not been redesigned yet.

## Korean-first navigation

Navigation is a macOS source list. The four primary views **오늘 · 받은 소식 · 탐색 · 보관함** sit at the top, extension groups follow, and **설정** sits at the bottom. Labels, feedback and application-facing errors use natural Korean; product identifiers and model names stay recognizable.

| View | Current behavior |
| --- | --- |
| 오늘 | Recent ranked notices with grounded reasons; cold-start recent items are explicitly not personalized. Stored current AI summaries are optional and do not influence ranking. |
| 받은 소식 | Full paginated chronological archive with unread/source/date/original-category filters and save controls. Recommendations never silently replace the inbox. |
| 탐색 | Explicit local title/body search, mixed Korean/English/numeric terms, plain-text snippets and highlighted matches. |
| 보관함 | User-saved notices, independent of extension enablement. Saving contributes only bounded local evidence and does not make attachments available automatically. |
| 설정 | Six panes, one at a time: 일반 (theme, power mode, start/close, automatic sync), 게시판 (extension switch, board tiles, collection), AI (engine, model tiles, analysis mode, 고급), 개인화 (profile, interest chips, learning, reset), 알림 (switch, OS permission, sensitivity, quiet hours, history) and 데이터 및 정보 (local data, backup/restore, about and updates). |

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

The extension switch represents persisted opt-in, not a background scheduler. Enabling **서강대학교 공지** reveals `SourceSettings.svelte`: the ten supported boards appear as tiles grouped by organization, and each tile is one large switch named "<organization> <board> 구독" (see [Settings](#settings)). The 22 **원문 수집 미지원** sources sit behind one "미지원 게시판 N개" disclosure as dashed, disabled tiles, never optimistically labeled working; the four Engineering boards count as supported only after full-article/local-resource verification.

**새로고침** runs both collection steps as one job for every subscribed supported board: it checks new notices and recent edits first, then continues an unfinished historical import from the saved checkpoint. A board's ⓘ popover offers the same action for that board alone; a completed history is not re-imported, so the former 다시 확인 button is gone. Subscribing alone starts nothing. The collection bar shows the current step (새 소식 확인 중, then 과거 자료 가져오는 중), board and page; each board's popover shows its status, history checkpoint, persisted attempt/success times, per-attempt discovered/archived/failed counts and last error, and the tile carries a warning mark (with a screen-reader description) while that error stands. Never-imported sources are not described as interrupted history; `limited` means partial work that can continue, not necessarily a university access restriction.

Historical collection can be cancelled and resumed. Limited, interrupted or failed work is not labeled complete. “Public historical pages complete” does not mean every attachment succeeded. The initial incremental seed is recent historical material, not a flood of new announcements. Incremental checks revisit recent/pinned records and discover newer records; they do not promise a full audit of every old edit.

A notice whose official body is only an iframe keeps its metadata and original fetched representation with empty sanitized body fields; the iframe is never fetched or rendered. The reader explains that the body is unsupported and offers to open the original in the browser. Such an observation never replaces an existing readable revision.

Disabling requests cancellation and waits for in-flight manual work, hides source controls and leaves stored text, resources, search and saved items accessible. Scheduler work is limited to enabled supported subscriptions and incremental collection, and stops when its extension/source eligibility is disabled. App-close-to-background is opt-in and requires a working tray; login launch and hidden login launch are opt-in. The scheduler is an in-process timer, not an OS service or wake alarm: no collection or notification processing occurs while the computer sleeps or after app exit. Actual sleep/wake catch-up behavior is not verified. AI analysis remains available for downloaded records when the extension is disabled.

## Lifecycle, scheduled synchronization and notifications

Settings puts lifecycle and scheduler controls in the **일반** pane and event policy and history in the **알림** pane. All lifecycle/scheduler/notification defaults are off. Close-to-background hides only when tray setup succeeded; otherwise closing quits. Login registration is changed only by explicit user action and checked against OS state. Hidden startup affects the OS login launch, not ordinary app launch.

The scheduler wakes every 15 seconds while the process lives. Source tasks follow automatic interval `0` (the default), which uses Eco/Balanced/Performance at 60/30/15 minutes, or an explicit 15-minute–12-hour interval that always remains exact, including 60 minutes. It runs incremental sync only, not historical backfill. It can be paused, observes extension/subscription changes, and records next-run/last-outcome/errors. Its timer is not guaranteed to fire while the operating system is asleep and does not wake the machine; sleep/wake and catch-up behavior are unverified.

Notification sensitivity modes are **important** (score at least 18 and user preferences exist), **interested** (explicit interest match), and **all**; negative document feedback and negative-interest matches always exclude. Quiet hours are optional local wall time, default values 22:00–07:00 while disabled. Multiple events may appear as one digest. Event history is local and contains links to archived revisions, statuses and policy/error reasons, not copied bodies. OS-facing notification text includes source name and notice title.

Permission status and permission requests are separate from the in-app switch. When the native status/request API is unavailable, Settings says so and disables the request rather than implying authorization or delivery. Notification history labels a handoff as unconfirmed and activation as opened; dismissals and expiry are not represented as clicks.

On macOS, **권한 요청** (accessible name 운영체제 알림 권한 요청) asks for permission only when pressed. Windows has no prompt: notifications can be enabled only when Windows settings allow them, and **시험 알림** (시험 알림으로 연결 확인) sends one test notification. Notification history shows the ten most recent items with **모두 보기** for the rest. AI does not run to prepare notifications. What has been observed natively is in [the roadmap](ROADMAP.md#verification-status).

## Local intelligence surfaces

First-run setup is a nonblocking app-shell banner with **로컬 AI 설정하기 / 나중에**, which opens the AI pane with its heading focused. It never hides the archive or starts a weight download by itself. Until setup completes, the pane leads with **로컬 AI 시작하기** and three steps (모델 설치, 불러오기, 동작 확인). The engine row shows the engine state, the loaded model and any fallback detail, with one primary action: **불러오기** or **메모리 해제**. Offered models (and every installed, external, retired or in-progress inventory row) are tiles showing name, use case and size; an installed tile is one large selection button, the chosen one tinted and marked 사용 중. A tile carries only its next action (설치, 중단, 이어받기, 모델 삭제/참조 해제); its ⓘ popover holds compatibility, download and memory estimates, ownership, file paths and catalog provenance/license/SHA-256. A collapsed **고급** area holds OS/architecture/physical RAM and refreshed disk capacity, the Windows execution device, engine restart, output length, the model installation folder and external GGUF registration; it opens by itself when the model folder reports an error. Physical RAM is not presented as currently available memory or a promise that loading will succeed. Compatibility labels distinguish qualified verification, experimental and unsupported artifacts. Exact Qwen3.5 2B/4B entries have macOS Apple Silicon/Metal evidence; a catalog entry that is not verified is not shown and cannot be installed with one click. An installed model whose catalog entry was removed (9B, 0.8B) stays under 내 모델 with a note that it is no longer offered and cannot be reinstalled or resumed; the user decides whether to delete it. These labels do not verify other backends/platforms or broad factual quality.

AI settings expose **모델 설치 폴더**, a native folder chooser and **기본 위치로 되돌리기**. The displayed path and free space refer to future downloads; existing models and interrupted downloads keep their original destination, shown in each model's ⓘ popover. Folder changes never move or delete weights. If a destination is unavailable, **파일을 남기고 목록에서 제거** permits reinstalling elsewhere without touching the old files.

On Windows only, **AI 실행 장치** offers automatic (NVIDIA GPU first, CPU fallback), CPU-only and NVIDIA-GPU-only choices. Selecting GPU is not proof it was used: engine status shows the actual backend and fallback reason, and saved analyses show **분석 당시 장치**. Platform-specific wording (menu bar vs. notification area, permission prompt vs. connection test, DMG vs. installer updates) follows the WebView user agent.

Model selection does not imply loaded memory. **선택한 모델 불러오기**, **모델 메모리 해제** and engine restart are separate actions. External GGUF registration explicitly references a full path without copying; reference removal never deletes external bytes. Managed deletion requires a second confirmation and says that documents/analyses remain. Setup completion performs a real short inference rather than treating download success as health. Backend-owned load/setup/register state shows the current phase and elapsed time, survives leaving and returning to Settings and offers cancellation. Cancellation may need to finish cooperative file verification or reap the worker before another operation starts; the UI does not announce success merely because a cancellation request was sent. Cancelled or failed setup does not persist completion.

The revision-keyed `AnalysisPanel.svelte` sits beside the reader header/body without replacing original content. It distinguishes intentional empty/no-model, pending/running, failure/retry, cached success, older model/prompt/schema/configuration and partial input. Revision-local job queries retain active work independently of bounded terminal history. Running analysis shows a backend phase and elapsed time, not a fabricated percentage; transient read failures offer retry and clear after successful refresh. Analyze/regenerate/cancel controls do not disable reading or saving. Existing results are accessible without the generating model. Old successful output remains visible when regeneration fails, and history retains model provenance.

Short summary and central AI category lead; detailed summary, extracted facts/dates and expandable **원문 근거** follow. A fact read from an archived image is labelled **이미지 근거** (never shown as a quotation) and its date says it came from an image and is not confirmed; the panel states how many archived images were used and, when some were skipped, that they were not analyzed. A model tile offers the optional image module only for an installed catalog model that pins one: add, progress and cancel, use with estimated memory, confirmed removal, or retry after failure; a text-only model shows no such control. AI classification is labeled separately from the original board category. Extracted dates are informational, not official calendar events. Partial results show actual selected/total character counts and warn that omitted intervals may contain important information. Long input selects bounded exact head/tail/important passages, deduplicating exact repeated text only when over budget and keeping structural table rows as separate evidence units. This is not complete-document understanding or chunk-and-merge. Generated fields render as escaped text, not HTML; evidence validation is not a promise of semantic accuracy.

Today’s recommendation list is independent of AI analysis. A separate section shows at most five real stored analyses, with their contract age labeled; the list remains usable if analysis lookup fails or no model is installed. Other archive rows use compact optional analysis indicators without requiring every item to be processed; search continues showing original matching snippets. There is no conversational transcript, glowing AI decoration, personalized “you must do this” claim, automatic calendar action or notification.

**AI 처리** defaults to explicit-only. The optional recent mode and per-open-session cap remain small. Candidates follow latest-revision collection time rather than publication date: new nonhistorical arrivals and meaningful revisions, including updates to historically collected documents, may qualify; initial backfill, startup history and previously attempted revisions do not. Advanced controls expose output length rather than dozens of sampling parameters. A queue lists status, failure and cancellation. Model/backend/native-quality verification is separate from UI implementation; see [AI evaluation](AI_EVALUATION.md).

## Settings

Settings follows macOS System Settings: a pane list on the left and one pane on the right, so a setting is found by choosing its pane rather than by scrolling. The pane list is a list of buttons with icons in the accent colour and `aria-current` on the shown pane. Each pane has one 20 px/700 heading; opening a pane from elsewhere (setup banner, update banner, sidebar board rows, Today's interest link, a grouped notification) focuses that heading or the named section heading. The last pane shown is kept while the app runs.

Content sits in **grouped rows**: rounded 10 px groups on `--pane` with a `--line` border, rows at least 46 px tall separated by hairlines, the title on the left (13 px/500) and one control on the right. Small group headings (13 px/650) sit above groups. A row carries a second line (12 px, `--secondary`) only for live state or a recovery instruction, never for explanation.

**The Title-Only Rule.** Rows, tiles and buttons show a title, not a description. An explanation a person may need (what a switch does, a limit, a caveat) goes in the row's ⓘ button, which opens a light-dismiss popover named "<title> 설명". Warnings that must not be missed stay visible: errors, confirmation dialogs (unchanged wording), permission recovery steps and destructive-action consequences.

Popovers use the native `popover` attribute (`Popover.svelte`), positioned under their button (above it when there is no room), closed by Escape, an outside click, scrolling or resizing, and returning focus to their button. They also hold detail views: a board's status and per-board actions, a model's provenance and paths, and the interest editor.

- **Board tiles ("light bulbs").** An unsubscribed tile is white with a hollow ring; a subscribed tile is lit, `--accent-fill` with white text, a filled check mark and a small accent shadow. The check mark and `aria-checked` carry the state, not colour alone. Unsupported tiles are dashed and disabled.
- **Model tiles** use the selection grammar instead: the chosen model gets `--soft`, a 2 px accent outline and "사용 중".
- **Interest chips.** 관심 and 관심 없음 rows hold explicit interests as pill buttons, each followed by a dashed "+ 추가" field (Enter adds with priority 3). 학습됨 holds learned interests as dashed pills. A chip opens its editor popover: name, aliases, a 1–5 priority and direction segmented control, then **저장** (learned: **직접 입력으로 저장**) and **삭제**. A draft typed while a save is in flight is kept.
- The profile is one row of three fields with placeholder labels and screen-reader labels.

At 640 px and narrower, row controls wrap below their title when they do not fit.

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

Layout is three panes on Today and two elsewhere. The sidebar is fixed at 232 px (200 px below 1100 px; a 60 px icon column below 800 px). Main content is padded 26 px top, 32 px sides and 40 px bottom (26 px sides below 1100 px; 22/20/32 px below 800 px). On Today a fixed 340 px week pane occupies the right edge and main content reserves that width. Today's blocks are separated by 28 px. Below 1100 px the week pane joins the main column (picks → week → recent analyses); at 640 px and narrower the feedback chips move below each pick. Settings uses a 176 px pane list (150 px below 1100 px; a wrapping row above the pane at 800 px and narrower) and a pane column up to 680 px; archive lists and reader share the same surfaces. Native window controls and a minimum window size are retained. The reader region is keyboard-focusable, has a Korean label/visible outline and scrolls wide content with horizontal arrows. Narrow native macOS reading and browser WebKit/Chromium 640 px/200% wide-table containment were checked. Browser results do not establish native WebView2 layout on Windows.

Depth is flat. Panes are separated by 1 px hairlines (`--line`) and the tonal step between `--bg`, `--sidebar` and `--pane`, not by shadows. The only shadows are small control affordances: the selected segment (`0 1px 3px #0000000a`), the switch knob (`0 1px 2px #0002`), the transient save notice (`0 4px 24px #0000000c`), the lit board tile (`0 3px 12px` of the accent at 28%) and floating popovers (`0 10px 32px` black at 14%, 45% in dark). Corners are 6 px for rows, buttons, chips and events, 8 px for pick rows and banners, 10 px for AI panels, settings groups, tiles and popovers, a full pill (13 px) for interest chips, and 2 px for swatches and event bars. Motion is native-quick: hover, selection and chip-reveal fades of 140–150 ms, a spinning icon only while a sync runs, and everything disabled under `prefers-reduced-motion`.

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
- **Do** give settings rows a title and one control, and put explanations in the row's ⓘ popover.
- **Don't** add eyebrows, letter-spaced kickers, header notes, taglines or stat tiles.
- **Don't** introduce a second chromatic accent beyond the family marks.
- **Don't** separate panes with shadows; use hairlines and tonal steps.
- **Don't** add description lines under settings rows, tiles or buttons; a second line is only for live state or a recovery step.

State is not communicated through color alone. Thin separators organize content without purple gradients, glowing AI ornaments or fabricated activity indicators.

## Interaction and accessibility boundaries

Theme selection applies immediately, persists through Rust and reverts on save failure; system mode follows OS preference. Energy profiles control app-open analysis budgets and idle model release, and also select scheduled-sync cadence when interval is automatic (`0`, the default). Eco starts no automatic AI analysis; Balanced and Performance allow bounded, opt-in recent work while the window is focused. Scheduled archive sync is independently opt-in; explicit intervals, including 60 minutes, remain exact. Effects are short and reduced-motion styles are included.

The interface includes labels, icon-button names, native keyboard-operable controls, focus-visible outlines, a skip link, landmarks, ordered headings and status/alert semantics. These are implementation properties, **not full accessibility conformance**.

Native review (Accessibility API inspection on macOS 27.2, v0.9.2 and v1.0.0, not spoken VoiceOver output) confirmed: the skip link and landmarks; one `h1` per page with ordered `h2`/`h3`; named pop-ups, fields, disclosures and per-item buttons; switch and toggle states; status/alert roles; the native confirmation sheet's title, message and buttons; tab order matching the visual order without a trap; focus moving to a notice heading on Return and back to the row on Escape; a visible dark-theme focus ring; names kept in the icon-only sidebar at 640 px. Theme token pairs meet 4.5:1 except `--muted` on `--soft` (4.41 light, 3.88 dark), which no element renders.

The redesigned shell (sidebar, page header, banners), Today and Settings were checked only with mocked-IPC Playwright captures in Chromium and WebKit; the native review above predates them. Their contrast values in [Color](#color) are computed, not measured. Not yet reviewed anywhere: spoken VoiceOver output and rotor, OS text size and zoom, increased contrast, OS-level reduced motion (reduced-motion CSS exists), WebView2 on Windows. There is no `prefers-contrast` styling, and the sidebar is opaque rather than vibrant.

## Beat branding

`assets/branding/beat.svg` is the original vector source. Four rounded bars of different heights form a rhythm recognizable at small sizes. Generated desktop icon formats in `src-tauri/icons/` remain committed. The redesign did not touch the source SVG or the desktop icon, which still use the earlier pre-redesign colours (`#F4F1EA` bars on `#344C43`); they are not part of the current palette and do not match the in-app accent.

In the app, the mark is a small 16 px-high set of four 3 px bars in `--accent`, next to the word "Rhythm" in the sidebar brand row. It follows the theme through the accent token. Branding can evolve without changing application boundaries. The original logo follows Rhythm's MIT license; font/dependency licenses and the rights to school content remain separate.
