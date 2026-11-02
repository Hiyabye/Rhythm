---
version: 1
slug: "src-app-svelte"
primary_target: "src/App.svelte"
related_targets: ["src/lib/ArchiveView.svelte"]
---

# Surface brief: app shell and Today

Scope: the window shell (sidebar, page header, global banners) and the 오늘 view. Mode: Operate. Other views inherit the shell and tokens; their content is out of scope this round.

Audience and job: the single user opens Rhythm from the menu bar or a notification and wants a day/week at a glance: what arrived this week, what to look at first and why, and AI reference dates (labeled, never ranking or alerting). The shell must leave a clear place for future extensions without showing fake entries.

Constraints: honest states (empty, cold start, failure, disabled extension, no AI), existing accessible names used by tests, Korean copy, keyboard focus return after the reader. Avoid: too sparse, too decorative.

Quality bar: Things 3 and Fantastical (user-named).

## Direction contract

THESIS: Rhythm reads as a native Mac app that sits beside Things 3 and Fantastical: source-list sidebar, a ranked Today list, and a week agenda. It refuses dashboard stat tiles, hero intros, taglines and web-template cards.

OWN-WORLD: macOS system grammar. Translucent-grey source-list sidebar with SF-style symbol icons, count badges and extension groups. White (light) or graphite (dark) content. Pretendard at system sizes with tabular numerals. One system-blue accent for selection, the unread dot and today's date circle. Small calendar-colour bars per institution (always paired with a text label). Hairline separators and inset list rows.

STORY: The user sees today's date and what's new, scans the ranked picks with their reasons, glances at the week to see arrivals and reference dates, then opens a notice or closes the window.

FIRST VIEWPORT: Three panes. Left: a 232px sidebar with 오늘 / 받은 소식 (unread badge) / 탐색 / 보관함 (saved badge), then one group per extension (서강대학교 공지 → subscribed boards; selecting one opens the inbox filtered to that board), with sync state and 설정 at the bottom. Centre: a 26px "오늘" title with the date and counts, then 나를 위한 소식 as inset rows (unread dot, title, source and date, reasons, optional AI summary, feedback buttons). Right: a 340px 이번 주 agenda, Monday to Sunday, with today circled, notice events and AI reference dates.

FORM: category standard (canon), chosen by the user. Not on the grounded list. seed:3b8a9276eef8a58f. Signature interaction: selecting a board in the sidebar scopes the inbox to that board. Motion grammar: native-quick (120–160ms) selection and hover fades only.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
