# Roadmap

Rhythm stays local-first and Korean-first. The current release is **v1.2.0**. Support targets macOS Apple Silicon and Windows 11 x64; Linux and Intel Macs are not supported. Planned work is tracked in [GitHub issues](https://github.com/Hiyabye/Rhythm/issues); this page records the current state and what has actually been verified.

## Current state

v1.0.0 (macOS, an ad-hoc-signed DMG) introduced the local notice archive, optional local AI analysis, deterministic personalization, opt-in background sync and notifications, and single-file backup/restore.

v1.1.0 adds:

- Windows 11 x64 support from contributor PR #1: per-user NSIS installer, CPU and NVIDIA Vulkan AI workers, WinRT notifications (schema 7), AI execution mode (schema 8). See [Windows](WINDOWS.md).
- A chosen folder for new model downloads on both platforms (schema 9).

v1.2.0 adds:

- Local AI runs in upstream `llama-server` (llama.cpp b12068, bundled and supervised by Rhythm) instead of the AI workers, on both platforms (#8, #61). Earlier results read as outdated until they are regenerated. On Windows it was observed on one GTX 1650 PC (Vulkan and CPU).
- Model catalog: Qwen3.5 0.8B added as experimental; 9B removed (existing installs keep working).
- Native-macOS redesign of the app shell and Today, and Settings split into panes.
- CI split by concern and gated by changed paths; a tag-triggered workflow (`release.yml`) that builds both platforms and creates a draft release. v1.1.0 is its first run.
- Release pipeline for in-app updates on unsigned builds: signed updater packages for both platforms and a verified `latest.json` in every release. v1.2.0 is the first release with the updater key; installed v1.1.0 apps embed no key and need one manual update.
- Automatic update checks, on by default in builds with the key (schema 10): daily while the app is open, announce-only, off switch in Settings → 데이터 및 정보.

## Verification status

Native observations are from one Mac (macOS 27.2, M1 Pro, 16 GiB) unless noted, mostly in isolated profiles or validation bundle identifiers. Windows observations are in [Windows validation](WINDOWS_VALIDATION.md); AI in [AI evaluation](AI_EVALUATION.md). Automated tests, mocked-IPC UI tests and native observations are separate claims.

| Area | Verified | Not verified |
| --- | --- | --- |
| Release build | v1.0.0 Metal DMG built locally: `hdiutil verify` valid, `codesign --verify --deep --strict` passes, `spctl` rejects (expected), arm64 only, no builder paths in binaries. Copied app ran on an isolated profile | A quarantined download and the "그래도 열기" flow; Finder/LaunchServices launch of the production identity (v0.9.2 tested LaunchServices with a validation identity); `release.yml` |
| Manual update | Replacing the app with a newer build kept the same profile, rows and resource hashes; one instance only | — |
| Archive | One article per supported source through the production path, then offline reopen with text, search, state and hash checks (repeated across releases). One small board (공과대학 학사공지, 5 notices) imported to its public end. Iframe-only notice `551681` kept metadata only | A full historical import across all supported sources; audits of old edits |
| Reading and search | Network-denied native reading, Korean search, local images, corrupted-image placeholder and explicit repair | Every source format |
| Personalization | Native ranking with explained reasons on synthetic and live notices, reversible feedback, learned memory, deletion and learning switch across restarts | — |
| Background | Close-to-background with zero windows; second launch restores the window; hidden-window scheduler run at the 30-minute default succeeded with no duplicates and no notifications from backfill; offline run persisted a 15-minute backoff | Sleep/wake; long-running use; clicking the tray menu |
| Login start | Temporary-HOME simulation: plist created and removed; `--autostart` with hidden start opened zero windows | A real user login in an installed app |
| Notifications | Isolated signed synthetic bundle: authorization prompt granted; a notification seen and clicked, matched by Accessibility and SQLite, not repeated after restart; quiet hours deferred three events, then a grouped alert delivered two while the rate limit held the third | Notifications from live sources; the production identity |
| Backup and restore | Populated native restore (25 of 26 tables identical; the intended model change) with quit and next-launch finalize; 10,000-notice synthetic release benchmark (backup 0.74 s lock + 15.8 s archive, restore stage 64.5 s, peak RSS ≈34 MB); real `ENOSPC` handled with no partial file; cancellation leaves nothing behind | Native backup/restore panels in v1.0.0; a real user profile larger than the synthetic one |
| Updater | macOS, validation identity, disposable key, loopback manifest. **First install:** a quarantined DMG's app, copied by Finder drag, was blocked by Gatekeeper and opened after "그래도 열기", untranslocated. **Update:** 1.1.0 → 1.1.1 from Settings: tampered archive rejected with the Korean signature message; genuine one replaced the bundle in place (ad-hoc + hardened runtime, no quarantine attribute), relaunched on the same profile; a later LaunchServices launch opened with no Gatekeeper prompt and no translocation; notification authorization stayed 허용됨. With a model loaded, `llama-server` was reaped before the old app exited. **Install folder not writable:** administrator prompt; cancel left the bundle intact (codesign valid) with the Korean permission message and no temp folders; approval replaced it (still user-owned, codesign valid) and relaunched. **Translocated or read-only location** (app copied by script and opened, or run from a disk image): update refused with the Korean move-to-Applications message, app kept running. **Automatic check** (validation build, loopback manifest): one request about a minute after launch, banner shown, success recorded; a relaunch the same day made no request; with the server stopped and the last check aged past a day, the failed check stayed silent and was not recorded. Local macOS `desktop:build` with a disposable key produced a `Rhythm.app.tar.gz` and `.sig` that pass `verify-update.mjs artifact` and `bundle`. **Windows** (one PC, validation identity, loopback): automatic check, tampered-update rejection and two passive NSIS updates with relaunch, `llama-server` reaped first, data kept; see [Windows validation](WINDOWS_VALIDATION.md#in-app-update) | Login item after update (the validation build shares the production `Rhythm.plist` LaunchAgent name, so it was not toggled; an in-place update keeps the path the plist points to); a real `/Applications` install; the full Windows progress window and a downloaded update file; `release.yml` with the production key; any published `latest.json` |
| Accessibility | Accessibility API review of v0.9.2 and v1.0.0: landmarks, headings, names, states, tab order, focus return, contrast pairs | Spoken VoiceOver output, OS text size, increased contrast and reduced motion; the redesigned shell and Today natively (only Playwright captures) |

Known performance note: restore staging flushes each file with `F_FULLFSYNC` (≈6 ms per file, ≈66 s for 11,081 files). Changing this is a durability decision.

## Open decisions

Each is a GitHub issue:

- **Cloud summaries for public notices** (#39 and follow-ups): precompute notice summaries on a backend and keep the local model for personal data. Not decided; the current rules forbid cloud inference.
- **Build and CI speed** (#9–#14, #17, #20, #23).

## Possible extensions

Each is a future product decision, sharing the archive, search and personalization core:

- **Cybercampus:** announcements, assignments, courses and deadlines, as a native Rust connector.
- **Opportunities:** competitions, activities, internships, research and jobs.
- **Calendar and tasks:** schedules, deadlines, reminders and daily planning.
- **Email:** after OAuth and provider limits are handled deliberately.
- **Personal agent:** proposals and workflows whose consequential actions stay under user control.

Platform work, also future decisions: Linux (re-review the `glib 0.18.5` / RUSTSEC-2024-0429 finding first), Intel Macs, Windows code signing, Developer ID signing and notarization.
