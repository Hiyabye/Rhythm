# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

(A Tauri 2 desktop app: the UI is Svelte 5 in the system webview, WebKit on macOS and WebView2 on Windows. Supported targets are macOS on Apple Silicon and Windows 11 x64. It is not a website and not a native iOS/Android app.)

## Users

One primary user: the author, a Sogang University computer science and engineering student, using Rhythm on their own Mac for their own day. Other people may install it, but they do not drive product decisions.

The job: never miss school information that matters, without checking a dozen university boards by hand, while keeping all personal data on the device.

Since v2.0.0 one central collector (a scheduled GitHub Actions job) gathers the public boards, embeds and classifies them once through Cloudflare Workers AI, and publishes a signed feed; every app mirrors the whole feed, so the server learns nothing about any user. Personal data (interests, feedback, saved and read state, todos, search queries, Cybercampus) never leaves the device.

## Product Purpose

Rhythm mirrors public Sogang notice boards from the signed feed into a local archive and fetches a notice's full body and files from the school when the user opens it. It ranks what matters to this user with explainable, deterministic rules, shows the feed's dates in a local calendar and the feed's categories and similar notices, and embeds search queries and Cybercampus notices with a small local embedding model. Archived content stays readable and searchable offline.

The direction is a personal planning hub: the place the user's day is organized around. Notices are the first data source. An opt-in Cybercampus connector adds the user's own course notices and deadlines. Possible later extensions (opportunities, calendar and tasks, a personal agent) share the same archive, search and personalization core. These extensions are possible future decisions, not current features.

Success: the user trusts Rhythm to bring forward what matters, at the right time, and rarely needs to visit school sites to check.

## Positioning

- **Local-first for everything personal.** No account, telemetry or cloud storage of user data. Public notices come from one signed feed that is identical for every user; the app never uploads anything. Search queries and Cybercampus notices are embedded on the device in a bundled llama.cpp server that only Rhythm starts and can reach.
- **Explainable, not oracular.** Recommendations are ranked by deterministic score signals, never by a model. Today keeps each pick to one line, so the signals are not shown there (user decision, 2026-11-07). AI output is optional and labeled. Its only effect on ranking is a small, capped bonus for notices similar to ones the user saved or marked interested, shown as a reason and off while AI is off; it never creates urgency.
- **User decides.** Subscriptions, collection, model installs, background sync and notifications are all off until the user turns them on. Learned preferences can be inspected, edited and deleted.
- **An archive, not just a feed reader.** Notices are preserved with meaningful revision history, local attachments and offline reading once opened. Fetch/parse failures never erase good content.

## Operating Context

The usage moments that matter most:

1. **Background + alerts.** Rhythm runs from the menu bar (the notification area on Windows) with the window closed. The feed sync (every 30 minutes while open), opt-in Cybercampus sync and native notifications pull the user in only when something matters (sensitivity modes, quiet hours, deduplication, rate limits).
2. **Planning hub.** Rhythm becomes the center of the day, especially as planning-oriented extensions arrive.

Also supported but secondary: a quick check of Today, deep reading of long notices with tables and attachments, and archive search.

Environment: a desktop window with native window controls and a 640 px minimum width, a tray item (menu bar on macOS, notification area on Windows), native notifications and native confirmation dialogs. The window may be hidden for long periods.

Current information architecture: every feature beyond the core shell is a built-in extension the user turns on, modeled on Obsidian's plugins (built-in only, never loaded from outside). The core is 오늘 and 설정 (확장 기능, 일반, 데이터 및 정보 with backup and updates). Extensions add 받은 소식 and its boards (서강대학교 공지), 과제·마감 (사이버캠퍼스), 탐색, 보관함, Today's 나를 위한 소식 and 앞으로 7일, AI 분류 (with the 기회 page) and 알림·백그라운드, each with its own settings pane. A new profile starts blank; upgraded profiles keep what they had.

## Capabilities and Constraints

- 34 cataloged public sources, all supported (main academic, five computing, four engineering, nine electronic/system semiconductor department, three dormitory, seven mechanical engineering and five semiconductor engineering boards), delivered through the feed, plus the opt-in Cybercampus course-notice source that the app collects itself with the user's own login. Unsupported sources, if any, are shown as such and are never presented as working.
- Public boards have no collect controls in the app: the board settings show the feed's age, the last check and each board's collector health, and a stale feed or a feed format the app cannot read is shown, never hidden. Cybercampus keeps **새로고침**; its progress shows real counts and phases, never fabricated percentages.
- Reader: sanitized local content only, with no scripts, iframes, remote images or live school pages. Original links open in the OS browser on explicit action. Attachments are revealed in Finder or File Explorer, never executed. Iframe-only notices keep metadata only and explain why, except that a school PDF-viewer body is shown as the PDF's extracted text with the PDF attached.
- AI: feed notices carry the collector's Qwen3-Embedding 0.6B vectors and categories (Workers AI), so categories and similar notices need no local model. The same model as a user-installed GGUF, run by a bundled llama.cpp server (Metal or CPU on macOS; CPU on Windows), embeds search queries for meaning-based search and classifies Cybercampus notices. AI summaries and the remote engine were retired (#226). Calendar dates come from deterministic text rules, not a model. Classification and dates can be wrong, and the UI must say to verify in the original.
- Personalization: explicit interests, per-notice interested/not interested feedback, learned topics from saves and positive feedback only, 180-day expiry, full reset.
- Lifecycle: close-to-background, login start, hidden login start, scheduler and notifications all default off. The automatic update check (builds with the updater key) defaults on by user decision; it only announces, and installing always needs confirmation.
- Single-file `.rhythm-backup` backup/restore (unencrypted, excludes model weights). Restore quits the app and finalizes on the next manual launch.
- Destructive actions require a confirmation that names what changes and what is kept.
- Distribution: ad-hoc-signed macOS DMG (not notarized) and unsigned per-user Windows installer. These are deliberate choices, not gaps. Releases carry Tauri-updater-signed packages for in-app updates, which the user confirms before installing; the in-app install path is verified only under validation identifiers with a loopback manifest (one Mac, one Windows PC), and builds without the key show manual steps.
- Frontend: Svelte 5 runes, TypeScript, Tailwind plus semantic CSS (`src/app.css`, `src/lib/ai.css`), Pretendard bundled locally (no font CDN). The Tauri webview cannot rely on remote assets.

## Brand Commitments

- **Native macOS look for the whole app (standing user decision, 2026-11-02).** In the redesign direction round, the user chose the category standard on purpose: a macOS-native app (source-list sidebar, system list rows, one system accent, light and dark themes) built without irony. Things 3 and Fantastical set the craft bar. This covers every screen, including Settings and the reader, not only the shell and Today. The same UI currently renders on Windows; whether Windows should get its own look has not been decided.
- Otherwise nothing is binding. The name "Rhythm", the Beat four-bar mark (`assets/branding/beat.svg`), the five-view navigation and the "command center" tagline are current but replaceable.

Voice that stays: all user-facing UI, feedback and errors are in natural Korean. Internal identifiers and model names stay recognizable. Copy is honest about limits: it never claims delivery, verification, completeness or accuracy that wasn't established.

## Evidence on Hand

- Real archived Sogang notices exist only in the user's local profile. Fixtures in `crates/sources/src/fixtures/` hold official notice content that keeps its original rights.
- No testimonials, user counts, benchmarks or press exist. Do not invent any.
- Measured on one machine (M1 Pro, 16 GiB): about 73 ms of Metal inference per synthetic notice with the Qwen3-Embedding model (237 ms on CPU), plus the one-time model integrity check. This is not a performance promise.
- Mocked-IPC Playwright UI tests (`npm run test:ui`) build and serve the real frontend for visual checks. They do not verify native behavior.

## Product Principles

1. **Quiet until it matters.** Rhythm spends most of its life in the background. Interruptions must be earned, and the open window should be calm, not a dashboard of alerts.
2. **Truth over reassurance.** Show real state: unconfirmed, partial, failed, stale, unsupported. Never fill gaps with fictional summaries, fake activity or success claims.
3. **The user stays in control.** Nothing collects, downloads, learns or notifies without an explicit choice, and every choice can be reversed.
4. **Grounded intelligence.** AI is an optional, labeled layer beside the original. Rankings and reasons come from deterministic signals.
5. **Built to grow into a hub.** New data sources and planning extensions should fit the same structure without redesigning the shell.

## Accessibility & Inclusion

Keep existing commitments: keyboard operation with visible focus, a skip link, landmarks and ordered headings, status/alert semantics, accessible names on icon buttons, no state communicated by color alone, 4.5:1 normal-text contrast in light and dark themes, and reduced-motion styles. Light, dark and system themes. Not yet verified: spoken VoiceOver output, OS text size/zoom and increased contrast.
