# Roadmap

Rhythm stays local-first and Korean-first. The current release is **v1.3.0**. Support targets macOS Apple Silicon and Windows 11 x64; Linux and Intel Macs are not supported. Planned work is tracked in [GitHub issues](https://github.com/Hiyabye/Rhythm/issues); this page records the current state and what has actually been verified.

## Current state

v1.0.0 (macOS, an ad-hoc-signed DMG) introduced the local notice archive, optional local AI analysis, deterministic personalization, opt-in background sync and notifications, and single-file backup/restore.

v1.1.0 adds:

- Windows 11 x64 support from contributor PR #1: per-user NSIS installer, CPU and NVIDIA Vulkan AI workers, WinRT notifications (schema 7), AI execution mode (schema 8). See [Windows](WINDOWS.md).
- A chosen folder for new model downloads on both platforms (schema 9).

v1.2.0 adds:

- Local AI runs in upstream `llama-server` (llama.cpp b12068, bundled and supervised by Rhythm) instead of the AI workers, on both platforms (#8, #61). Earlier results read as outdated until they are regenerated. On Windows it was observed on one GTX 1650 PC (Vulkan and CPU).
- Model catalog: Qwen3.5 2B/4B and Gemma 4 E2B/E4B verified on macOS Apple Silicon / Metal; 9B and 0.8B removed (existing installs keep working).
- Native-macOS redesign of the app shell and Today, and Settings split into panes.
- CI split by concern and gated by changed paths; a tag-triggered workflow (`release.yml`) that builds both platforms and creates a draft release. v1.1.0 is its first run.
- Release pipeline for in-app updates on unsigned builds: signed updater packages for both platforms and a verified `latest.json` in every release. v1.2.0 is the first release with the updater key; installed v1.1.0 apps embed no key and need one manual update.
- Automatic update checks, on by default in builds with the key (schema 10): daily while the app is open, announce-only, off switch in Settings → 데이터 및 정보.

v1.3.0 adds:

- **새로고침** merges the new-notice check and the earlier-notice import into one action (#73). Notices whose body cannot be archived are skipped and listed per board instead of blocking collection (#78).
- The 과거 자료 concept is gone. Opt-in "analyze older notices while idle" (#75, schema 14) analyzes up to 20 unanalyzed notices per hour while the computer is idle on external power, newest publication first.
- Model catalog: Qwen3.5 0.8B removed because it invented content (#74; existing installs keep working). Gemma 4 E2B/E4B verified on macOS Apple Silicon / Metal.
- Local embedding classification (#76, schema 15): Qwen3-Embedding 0.6B Q8_0 as a separate explicit install and transient `llama-server` process. It classifies the existing archive and new revisions, with a low-confidence `기타` fallback. Category is removed from the generation schema (prompt 5 / schema 3); earlier analyses remain readable as outdated. Measured on macOS Metal and CPU with 18 synthetic fixtures; Windows and broader accuracy are unverified.
- Optional vision projector (#111): an installed `mmproj` adds up to two already-downloaded images of a notice to its analysis; image facts are marked as read from an image and ambiguous dates stay unknown.
- Notice images may be downloaded from any public HTTPS host (that host's robots.txt applies), and 자료 모두 내려받기 fetches every missing file of a board (#104).
- The login item is named after the bundle identifier, so a validation build no longer shares the production entry (#67). A failed analysis shows a Korean user-facing error instead of the model's repair instruction (#98).

## Verification status

Native observations are from one Mac (macOS 27.2, M1 Pro, 16 GiB) unless noted, mostly in isolated profiles or validation bundle identifiers. Windows observations are in [Windows validation](WINDOWS_VALIDATION.md); AI in [AI evaluation](AI_EVALUATION.md). Automated tests, mocked-IPC UI tests and native observations are separate claims.

| Area | Verified | Not verified |
| --- | --- | --- |
| Release build | v1.0.0 Metal DMG built locally: `hdiutil verify` valid, `codesign --verify --deep --strict` passes, `spctl` rejects (expected), arm64 only, no builder paths in binaries. Copied app ran on an isolated profile | A quarantined download and the "그래도 열기" flow; Finder/LaunchServices launch of the production identity (v0.9.2 tested LaunchServices with a validation identity); `release.yml` |
| Manual update | Replacing the app with a newer build kept the same profile, rows and resource hashes; one instance only | — |
| Archive | One article per supported source through the production path (34 sources, including the seven sitemap-backed mechanical boards and the five semiconductor boards on 2026-11-06), then offline reopen with text, search, state and hash checks (repeated across releases). One small board (공과대학 학사공지, 5 notices) imported to its public end. Iframe-only notice `551681` kept metadata only | A full historical import across all supported sources; audits of old edits |
| Reading and search | Network-denied native reading, Korean search, local images, corrupted-image placeholder and explicit repair | Every source format |
| Personalization | Native ranking with explained reasons on synthetic and live notices, reversible feedback, learned memory, deletion and learning switch across restarts | — |
| Background | Close-to-background with zero windows; second launch restores the window; hidden-window scheduler run at the 30-minute default succeeded with no duplicates and no notifications from backfill; offline run persisted a 15-minute backoff | Sleep/wake; long-running use; clicking the tray menu |
| Login start | Temporary-HOME simulation: plist created and removed; `--autostart` with hidden start opened zero windows | A real user login in an installed app |
| Notifications | Isolated signed synthetic bundle: authorization prompt granted; a notification seen and clicked, matched by Accessibility and SQLite, not repeated after restart; quiet hours deferred three events, then a grouped alert delivered two while the rate limit held the third | Notifications from live sources; the production identity |
| Backup and restore | Populated native restore (25 of 26 tables identical; the intended model change) with quit and next-launch finalize; 10,000-notice synthetic release benchmark (backup 0.74 s lock + 15.8 s archive, restore stage 64.5 s, peak RSS ≈34 MB); real `ENOSPC` handled with no partial file; cancellation leaves nothing behind | Native backup/restore panels in v1.0.0; a real user profile larger than the synthetic one |
| Updater | macOS, validation identity, disposable key, loopback manifest. **First install:** a quarantined DMG's app, copied by Finder drag, was blocked by Gatekeeper and opened after "그래도 열기", untranslocated. **Update:** 1.1.0 → 1.1.1 from Settings: tampered archive rejected with the Korean signature message; genuine one replaced the bundle in place (ad-hoc + hardened runtime, no quarantine attribute), relaunched on the same profile; a later LaunchServices launch opened with no Gatekeeper prompt and no translocation; notification authorization stayed 허용됨. With a model loaded, `llama-server` was reaped before the old app exited. **Install folder not writable:** administrator prompt; cancel left the bundle intact (codesign valid) with the Korean permission message and no temp folders; approval replaced it (still user-owned, codesign valid) and relaunched. **Translocated or read-only location** (app copied by script and opened, or run from a disk image): update refused with the Korean move-to-Applications message, app kept running. **Automatic check** (validation build, loopback manifest): one request about a minute after launch, banner shown, success recorded; a relaunch the same day made no request; with the server stopped and the last check aged past a day, the failed check stayed silent and was not recorded. Local macOS `desktop:build` with a disposable key produced a `Rhythm.app.tar.gz` and `.sig` that pass `verify-update.mjs artifact` and `bundle`. **Windows** (one PC, validation identity, loopback): automatic check, tampered-update rejection and two passive NSIS updates with relaunch, `llama-server` reaped first, data kept; see [Windows validation](WINDOWS_VALIDATION.md#in-app-update). **1.2.0 → 1.3.0** (validation identity, disposable key, loopback manifest): the bundle at the same path was replaced in place and relaunched; `codesign --verify --deep --strict` passes, still ad-hoc + hardened runtime, no quarantine attribute; the profile (settings row and a registered external model) survived; the login LaunchAgent still pointed at the same executable path, and the identifier-named entry from #67 was created on demand after the update; with a model loaded, `llama-server` disappeared before the app restarted; a read-only install folder showed the administrator prompt and, on cancel, the Korean permission message with the bundle intact. **Published release** (v1.3.0): the stable `latest.json` serves 1.3.0, its assets verify against the repository `RHYTHM_UPDATER_PUBKEY` (`verify-update.mjs manifest`, `bundle` for the macOS archive, `artifact` for the Windows installer), and both `.sha256` files match the downloaded files | Notification authorization surviving an update in the 1.2.0 → 1.3.0 run (macOS showed no authorization prompt for the ad-hoc validation build: the app's 20-second wait timed out and the status became 거부됨, so permission survival was not re-observed; the 1.1.0 → 1.1.1 run did observe 허용됨); a real `/Applications` install; the full Windows progress window and a downloaded update file; the live update path from an installed 1.2.0 app through GitHub |
| Classification | macOS Metal and CPU on 18 synthetic fixtures (18/18; 73 ms and 237 ms per notice; 1.7/2.2 GiB peak). Isolated validation-identity app with a seeded profile classified 4 notices in 18 s through the real background loop | Windows CPU/Vulkan; broader Korean accuracy; a native Settings/reader visual check |
| Accessibility | Accessibility API review of v0.9.2 and v1.0.0: landmarks, headings, names, states, tab order, focus return, contrast pairs | Spoken VoiceOver output, OS text size, increased contrast and reduced motion; the redesigned shell and Today natively (only Playwright captures) |
| Remote AI | Fake OpenAI-compatible endpoint: URL policy, request-body privacy, schema fallback, errors without key echo, cancellation, cache separation, automatic cap; mocked-IPC settings/reader tests | Any real provider or model (structured output, Korean quality, latency, limits, cost); remote embedding accuracy; native Keychain/Credential Manager prompts |

Known performance note: restore staging flushes each file with `F_FULLFSYNC` (≈6 ms per file, ≈66 s for 11,081 files). Changing this is a durability decision.

## Open decisions

Each is a GitHub issue:

- **Cloud summaries for public notices** (#39 and follow-ups): precompute notice summaries on a backend and keep the local model for personal data. Not decided. The shipped opt-in user-configured remote engine (#77) is a separate client and does not authorize a Rhythm-operated backend.
- **Build and CI speed** (#9–#14, #17, #20, #23).

## Possible extensions

Each is a future product decision, sharing the archive, search and personalization core:

- **Cybercampus:** announcements, assignments, courses and deadlines, as a native Rust connector.
- **Opportunities:** competitions, activities, internships, research and jobs.
- **Calendar and tasks:** schedules, deadlines, reminders and daily planning. The
  dormitory calendar (`/sgdormitory/6558/subview.do`) is a month grid of title/time
  schedule entries, not notices; supporting it needs this event model rather than the
  notice archive.
- **Email:** after OAuth and provider limits are handled deliberately.
- **Personal agent:** proposals and workflows whose consequential actions stay under user control.

Platform work, also future decisions: Linux (re-review the `glib 0.18.5` / RUSTSEC-2024-0429 finding first), Intel Macs, Developer ID signing and notarization. Windows builds stay unsigned by user decision (#85); SmartScreen warnings remain expected.
