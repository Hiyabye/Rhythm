# Roadmap

Rhythm stays local-first and Korean-first. The current release is **v1.1.0**. Support targets macOS Apple Silicon and Windows 11 x64; Linux and Intel Macs are not supported. Planned work is tracked in [GitHub issues](https://github.com/Hiyabye/Rhythm/issues); this page records the current state and what has actually been verified.

## Current state

v1.0.0 (macOS, an ad-hoc-signed DMG) introduced the local notice archive, optional local AI analysis, deterministic personalization, opt-in background sync and notifications, and single-file backup/restore.

v1.1.0 adds:

- Windows 11 x64 support from contributor PR #1: per-user NSIS installer, CPU and NVIDIA Vulkan AI workers, WinRT notifications (schema 7), AI execution mode (schema 8). See [Windows](WINDOWS.md).
- A chosen folder for new model downloads on both platforms (schema 9).

Unreleased on `main`: local AI runs in upstream `llama-server` (llama.cpp b12068, bundled and supervised by Rhythm) instead of the AI workers, on both platforms (#8, #61). Earlier results read as outdated until they are regenerated. Windows inference with it is not yet verified.
- Model catalog: Qwen3.5 0.8B added as experimental; 9B removed (existing installs keep working).
- Native-macOS redesign of the app shell and Today.
- CI split by concern and gated by changed paths; a tag-triggered workflow (`release.yml`) that builds both platforms and creates a draft release. v1.1.0 is its first run.

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
| Updater | Validation identity with a disposable key and loopback manifest: tampered archive rejected, genuine update installed and relaunched on the same profile | Any production key or release (none exists by policy) |
| Accessibility | Accessibility API review of v0.9.2 and v1.0.0: landmarks, headings, names, states, tab order, focus return, contrast pairs | Spoken VoiceOver output, OS text size, increased contrast and reduced motion; the redesigned shell and Today natively (only Playwright captures) |

Known performance note: restore staging flushes each file with `F_FULLFSYNC` (≈6 ms per file, ≈66 s for 11,081 files). Changing this is a durability decision.

## Open decisions

Each is a GitHub issue:

- **Cloud summaries for public notices** (#39 and follow-ups): precompute notice summaries on a backend and keep the local model for personal data. Not decided; the current rules forbid cloud inference.
- **In-app updates on unsigned builds** (#30 and follow-ups): Tauri updater signatures without OS code signing.
- **Build and CI speed** (#9–#14, #17, #20, #23).

## Possible extensions

Each is a future product decision, sharing the archive, search and personalization core:

- **Cybercampus:** announcements, assignments, courses and deadlines, as a native Rust connector.
- **Opportunities:** competitions, activities, internships, research and jobs.
- **Calendar and tasks:** schedules, deadlines, reminders and daily planning.
- **Email:** after OAuth and provider limits are handled deliberately.
- **Personal agent:** proposals and workflows whose consequential actions stay under user control.

Platform work, also future decisions: Linux (re-review the `glib 0.18.5` / RUSTSEC-2024-0429 finding first), Intel Macs, Windows code signing, Developer ID signing and notarization.
