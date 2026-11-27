# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

Rhythm is a Tauri 2 desktop app. The UI is Svelte 5 in the system webview (WebKit on macOS, WebView2 on Windows). Supported targets are macOS on Apple Silicon and Windows 11 x64. It is not a website and not a native iOS/Android app.

## Users

One primary user: the author, a Sogang University computer science and engineering student, using Rhythm on their own Mac for their own day. Other people may install it, but they do not drive product decisions.

The job: never miss school information that matters, without checking a dozen university boards by hand, and keep all personal data on the device.

One central collector (a scheduled GitHub Actions job) gathers the public boards, embeds and classifies them once, and publishes a signed feed. Every app mirrors the whole feed, so the server learns nothing about any user. Interests, feedback, saved and read state, todos, search queries and Cybercampus data never leave the device.

## Product Purpose

Rhythm mirrors public Sogang notice boards from the signed feed into a local archive. It fetches a notice's full body and files from the school when the user opens it. Archived notices stay readable and searchable offline.

It ranks what matters to this user with explainable, deterministic rules. It shows the feed's dates in a local calendar, and the feed's categories and similar notices. A small local embedding model handles search queries and Cybercampus notices.

The direction is a personal planning hub, the place the user's day is organized around. Notices come first. Opt-in extensions add more on the same archive, search and personalization core:

- Cybercampus: the user's own course notices and deadlines.
- Opportunities (기회), a calendar with todos, and Studium seat checks.

A personal agent is a possible future decision, not a current feature.

Success: the user trusts Rhythm to bring forward what matters at the right time, and rarely needs to visit school sites.

## Positioning

- **Local-first for everything personal.** No account, telemetry or cloud storage of user data. Public notices come from one signed feed that is the same for every user, and the app uploads nothing. Embedding runs on the device in a bundled llama.cpp server that only Rhythm starts and can reach.
- **Explainable, not oracular.** Deterministic score signals rank recommendations, never a model. Today shows one line per pick, so it does not show the signals (user decision, 2026-11-07). AI output is optional and labeled. Its only effect on ranking is a small, capped bonus for notices similar to ones the user saved or marked interested. That bonus is shown as a reason, is off while AI is off, and never creates urgency.
- **User decides.** Subscriptions, collection, model installs, background sync and notifications are off until the user turns them on. The user can inspect, edit and delete learned preferences.
- **An archive, not just a feed reader.** Notices keep meaningful revision history and local attachments. Fetch or parse failures never erase good content.

## Operating Context

The moments that matter most:

1. **Background and alerts.** Rhythm runs from the menu bar (the notification area on Windows) with the window closed. Feed sync, the opt-in Cybercampus sync and native notifications interrupt the user only when something matters. Sensitivity modes, quiet hours, deduplication and rate limits control this.
2. **Planning hub.** Rhythm is the center of the day, more so as planning extensions arrive.

Secondary: a quick look at Today, deep reading of long notices with tables and attachments, and archive search.

Environment: a desktop window with native window controls and a 640 px minimum width, a tray item, native notifications and native confirmation dialogs. The window may stay hidden for long periods.

Information architecture: every feature beyond the core shell is a built-in extension the user turns on, like Obsidian's plugins. Extensions are built in and never loaded from outside.

- The core is 오늘 and 설정 (확장 기능, 일반, 데이터 및 정보 with backup and updates).
- Extensions add 받은 소식 and its boards (서강대학교 공지), 과제·마감 (사이버캠퍼스), 검색, 보관함, Today's 나를 위한 소식 and 앞으로 7일, 기회 (AI 분류), 캘린더, 열람실 and 알림·백그라운드.
- Boards, Cybercampus, personalization, AI and 알림·백그라운드 add settings panes.
- 휴지통 shows while a source extension is on or while it still holds notices.
- A new profile starts blank. Upgraded profiles keep what they had.

## Capabilities and Constraints

- **Sources:** 172 cataloged public boards (catalog: `crates/sources/AGENTS.md`), delivered through the feed. The opt-in Cybercampus source is collected by the app with the user's own login. Unsupported sources are shown as unsupported, never as working.
- **Board controls:** Public boards have no collect controls in the app. Board settings show the feed's age, the last check and each board's collector health. A stale feed or an unreadable feed format is shown, never hidden. Cybercampus keeps **새로고침**, and its progress shows real counts and phases, never made-up percentages.
- **Reader:** Sanitized local content only. No scripts, iframes, remote images or live school pages. Original links open in the OS browser on explicit action. Attachments are revealed in Finder or File Explorer, never executed. Iframe-only notices keep metadata only and say why. The exception is a school PDF-viewer body, which shows the PDF's extracted text with the PDF attached.
- **AI:** Feed notices carry the collector's Qwen3-Embedding 0.6B vectors and categories, so categories and similar notices need no local model. The same model, installed by the user as a GGUF and run by the bundled llama.cpp server (Metal or CPU on macOS, CPU on Windows), embeds search queries for meaning-based search and classifies Cybercampus notices. AI summaries and the remote engine are retired. Calendar dates come from deterministic text rules, not a model. Classification and dates can be wrong, and the UI must say to check the original.
- **Personalization:** Explicit interests, per-notice interested/not interested feedback, topics learned only from saves and positive feedback, 180-day expiry, full reset.
- **Lifecycle:** Close-to-background, login start, hidden login start, the scheduler and notifications are all off by default. The automatic update check (only in builds with the updater key) is on by default by user decision. It only announces, and installing always needs confirmation.
- **Backup:** One `.rhythm-backup` file, unencrypted, without model weights. Restore quits the app and finishes on the next manual launch.
- **Destructive actions** need a confirmation that names what changes and what is kept.
- **Distribution:** An ad-hoc-signed macOS DMG (not notarized) and an unsigned per-user Windows installer. These are deliberate choices, not gaps. Releases carry Tauri-updater-signed packages for in-app updates, which the user confirms before installing. The in-app install path is verified only under validation identifiers with a loopback manifest (one Mac, one Windows PC). Builds without the key show manual steps.
- **Frontend:** Svelte 5 runes, TypeScript, Tailwind plus semantic CSS (`src/app.css`), Pretendard bundled locally with no font CDN. The webview cannot rely on remote assets.

## Brand Commitments

- **Native macOS look for the whole app (standing user decision, 2026-11-02).** The user chose the category standard on purpose: a macOS-native app with a source-list sidebar, system list rows, one system accent and light and dark themes, built without irony. Things 3 and Fantastical set the craft bar. This covers every screen, including Settings and the reader. The same UI renders on Windows. Whether Windows gets its own look is undecided.
- Nothing else is binding. The name "Rhythm", the Beat four-bar mark (`assets/branding/beat.svg`), the source-list navigation and the "command center" tagline are current but replaceable.

Voice: all user-facing UI, feedback and errors are in natural Korean. Internal identifiers and model names stay recognizable. Copy is honest about limits. It never claims delivery, verification, completeness or accuracy that was not established.

## Evidence on Hand

- Real archived Sogang notices exist only in the user's local profile. Fixtures in `crates/sources/src/fixtures/` hold official notice content that keeps its original rights.
- No testimonials, user counts, benchmarks or press exist. Do not invent any.
- Inference speed was measured on one machine only ([AI evaluation](docs/AI_EVALUATION.md)). It is not a performance promise.
- Mocked-IPC Playwright UI tests (`npm run test:ui`) build and serve the real frontend for visual checks. They do not verify native behavior.

## Product Principles

1. **Quiet until it matters.** Rhythm spends most of its life in the background. Interruptions must be earned. The open window should be calm, not a dashboard of alerts.
2. **Truth over reassurance.** Show real state: unconfirmed, partial, failed, stale, unsupported. Never fill gaps with made-up summaries, fake activity or success claims.
3. **The user stays in control.** Nothing collects, downloads, learns or notifies without an explicit choice, and every choice can be reversed.
4. **Grounded intelligence.** AI is an optional, labeled layer beside the original. Rankings and reasons come from deterministic signals.
5. **Built to grow into a hub.** New data sources and planning extensions should fit the same structure without redesigning the shell.

## Accessibility & Inclusion

Keep these commitments:

- Keyboard operation with visible focus, a skip link, landmarks and ordered headings.
- Status and alert semantics, and accessible names on icon buttons.
- No state shown by color alone, and 4.5:1 normal-text contrast in light and dark themes.
- Reduced-motion styles, and light, dark and system themes.

Not yet verified: spoken VoiceOver output, OS text size and zoom, and increased contrast.
