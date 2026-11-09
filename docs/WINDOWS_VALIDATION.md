# Windows validation

What has actually been observed on Windows 11 x64. Build setup is in [Windows](WINDOWS.md).

## Sources of evidence

- **Contributor PC (PR #1 and PR #2), 2026-11-02 to 2026-11-03.** One Windows 11 Home x64 PC (build 26300, Intel Core i5-10300H, 16 GB RAM, NVIDIA GTX 1650 4 GiB, driver 552.44). Most installed-app observations used the port's old identifier `app.rhythm.windows` and builds 0.9.2–0.9.4, before the merge into the shared `io.github.hiyabye.rhythm` codebase. They do not verify the current build.
- **Contributor PC, PR #63 (`llama-server`), 2026-11-03.** The same PC. Model, socket and installed-app runs used commit `bb14e24`; workspace Clippy and tests were re-run on `8354512`, which only restricts one Metal test to macOS.
- **Contributor PC, #36 (in-app update), 2026-11-04.** The same PC (NVIDIA driver 31.0.15.5244), commit `fa86dad`, a fresh clone with `core.autocrlf=false`. Builds used a validation identifier, a disposable updater key and a loopback manifest; the evidence archive is attached to issue #36.
- **GitHub CI (`windows-latest`).** Every pull request and push to `main` that touches Rust runs Clippy and tests for the workspace on a hosted Windows Server runner, including GPU fallback tests against a fake `llama-server` and a lifecycle test of the staged real builds without model weights. `main` also builds the NSIS installer when packaging inputs change. CI has no NVIDIA GPU, so these tests check supervision and fallback, not inference.

No second PC or VM has been tested. The only installed runs of the current codebase are the PR #63 and #36 builds under validation identifiers (see [AI on Windows](#ai-on-windows) and [In-app update](#in-app-update)).

## Automated checks on the contributor PC (PR #2, shared identifier)

| Check | Result |
| --- | --- |
| `cargo fmt --check`, workspace Clippy with `-D warnings` | Passed |
| `cargo test --locked --workspace` with the CPU worker set in `RHYTHM_WORKER_TEST_BIN` | App 348 passed, 13 ignored; CPU worker 10 unit + 4 process passed |
| GPU worker build, Clippy and tests | 14 unit + 4 process passed |
| Cloud Files preflight (`-CheckOnly`) | `0x80070186` for the local temp folder; native path test passed for Korean/space and verbatim paths |
| Restore process-abort test at each transition | Passed. Process abort is not power-loss validation |

## Installed-app observations (old identifier, builds 0.9.2–0.9.4)

| Area | Observed | Not verified |
| --- | --- | --- |
| Install and update | 0.9.2 per-user installer installed; a 0.9.1 fixture upgraded to 0.9.2 with all 10 documents and 11 resource hashes intact | Uninstall and data retention of the final build; protocol/startup cleanup |
| Archive | Bounded live import of one document per supported source (10 documents, 11 resource references, 7 distinct files); offline reopen, integrity and hash checks passed. Two `eng-research` images on `ee.sogang.ac.kr` were blocked by the allowlist as intended | Full history; Korean IME search; revisions; resource reveal after its fix |
| Window, tray, login | Login toggle wrote the quoted executable plus `--autostart` to the per-user Run key; close-to-background hid the window with app and worker running; a second launch restored the same process; manual `--autostart` started hidden | A real login, the tray menu actions, sleep/wake, scheduled sync |
| Notifications | Shortcut AUMID, CLSID and protocol registration verified; one neutral test toast handoff returned success | Visible display, click navigation and one-use tokens in running, hidden and exited states (click tests deferred by the user) |
| Backup and restore | UI preview, cancel, confirmed restore with app exit and manual relaunch; empty and populated isolated profiles restored with integrity and hash checks; previous profile retained | Model reconciliation and token invalidation in the installed app; power loss |
| Model folder (0.9.4) | Native folder picker chose a Korean path; the choice persisted across restart; reset showed the default `models` folder | Large downloads to a chosen folder in the installed release |
| Accessibility | Not tested | WebView2 scaling, keyboard/focus, screen reader, contrast, reduced motion |

## AI on Windows

Observed with the bundled `llama-server` b12068 (PR #63). Model runs used the pinned 2B/4B Q4_K_M files copied from existing local copies (size and SHA-256 matched); no weights were downloaded.

| Check | Observed |
| --- | --- |
| Staging | `stage-llama.mjs` downloaded the Vulkan and CPU archives; size and SHA-256 matched the pins. VC runtime DLLs version 14.44.35211.0, Microsoft signature valid |
| Workspace Clippy and tests (`8354512`) | Passed: 369 passed, 13 ignored. The 5 `gpu_tests` (fake server) and `real_server_lifecycle_without_model_weights` (real b12068, not skipped) passed |
| Vulkan device and offload | `using device Vulkan1 (NVIDIA GeForce GTX 1650)`; `offloaded 25/25` (2B) and `33/33` (4B) layers |
| Unix socket | `llama-server --host <path>.sock` served `GET /health` over a Unix domain socket, with and without a loaded 2B model. The app still uses loopback on Windows |
| Default `npm run desktop:build` | Succeeded and produced `Rhythm_1.1.0_x64-setup.exe` |
| Installed app (validation identifier) | A rebuild with identifier `io.github.hiyabye.rhythm.pr63.validation` and an isolated `RHYTHM_VALIDATION_DATA_DIR` installed; all 55 files under `llama/cpu` and `llama/vulkan` matched the staged hashes. In the real window: registered the 2B file, completed setup in `gpu` mode, analyzed one synthetic scholarship notice on GPU (`loaded_backend` vulkan, 12.6 s), switched to `cpu` and analyzed it again (cpu, 61.0 s). Both results had the right category and deadline; the GPU result was marked as differing from the current setting. The app used loopback, and no app or `llama-server` process remained after closing the window |

Synthetic evaluation (`actual_qwen35_analysis_evaluation`, release build, production controller → validation → Store path). The harness always uses `auto`, so the CPU runs used a resource folder without the Vulkan build; the backend column is the reported `loaded_backend`. Durations are the sum of job wall times, and the first 2B GPU run overlapped a release build.

| Model / backend | Completed | Passed every screening check | Total |
| --- | ---: | ---: | ---: |
| 2B / Vulkan | 18/18 | 8/18 | 262 s |
| 4B / Vulkan | 18/18 | 14/18 | 441 s |
| 2B / CPU | 18/18 | 7/18 | 941 s |
| 4B / CPU (first 4 fixtures only) | 4/4 | 4/4 | 537 s |

Screening failures were wrong categories (mostly `학사`), missing or extra deadlines and missing summary terms, as on macOS.

Not verified: other NVIDIA GPUs or drivers, the production-identifier install with protocol/notification registration, uninstall and upgrade, 4B in the installed app, model cancellation or crash recovery in the installed app, VRAM exhaustion, memory limits, peak RAM/VRAM, OS-enforced offline operation.

## In-app update

Observed in #36: build A (1.1.0) and build B (1.1.1, updater-signed with a disposable key), both with identifier `io.github.hiyabye.rhythm.validation`, served from `127.0.0.1` over HTTP. This is the Tauri updater signature, not Authenticode or SmartScreen trust.

| Check | Observed |
| --- | --- |
| `npm run check`, `npm run test:release` | 0 errors and warnings; 24/24 passed in the LF clone. In a default `core.autocrlf=true` checkout one workflow test failed on CRLF line endings (test fixed since) |
| Independent signature checks | `verify-update.mjs` accepted B, rejected B with one byte flipped, and found the disposable key embedded in B's binary |
| First install of A | Installed per-user to `%LOCALAPPDATA%\Programs\Rhythm`; first launch opened an empty validation profile at schema 10. No SmartScreen or UAC prompt for the locally built installer, which had no `Zone.Identifier` stream |
| Automatic check | First `latest.json` request about 63 s after launch; the 1.1.1 banner appeared before any manual check |
| Tampered update | Rejected with the Korean signature message; A stayed 1.1.0 with an unchanged executable hash and the loaded `llama-server` kept running |
| Genuine update, twice | Check, download and "설치 및 다시 시작" installed B and relaunched it automatically with no further input. In the second run a GPU model was loaded (Vulkan, GTX 1650): `llama-server` exited about 670 ms before the updater process (`Rhythm-1.1.1-installer`) started, and the new app started 7 s later |
| Installed files | Executable version 1.1.1; it differs from the build output only by Tauri's 3-byte NSIS bundle-type marker. All 58 bundled runtime files (55 llama.cpp, 3 VC runtime) matched the staged hashes; A and B share the same pins, so no runtime replacement was exercised |
| Data | Both updates kept every `settings`, `ai_preferences` and `ai_models` row, the main table counts, the external model reference and selection; `integrity_check` ok, `foreign_key_check` empty |
| Shortcuts | The desktop shortcut from the first install was unchanged and not duplicated; the Start menu shortcut still pointed at the installed executable |
| Uninstall | The validation uninstaller exited 0 and kept the validation profile |

Not verified: the whole NSIS progress window (the frames were not all captured, so warnings or prompts cannot be ruled out); a downloaded (`Zone.Identifier`) update file; launching through the shortcuts; the About version text in B (it is fixed in source, so the version override does not change it); a populated archive; a runtime pin change between versions; the production identifier, the GitHub endpoint and `release.yml` with the production key; other PCs and GPUs.

## Studium

Contributor PC, 2026-11-09, on the Studium branch before it was split into PRs #182–#186: `npm run check` (zero errors/warnings), frontend production build, Rustfmt and locked all-target Clippy with `-D warnings` passed. The full locked Rust run reported **572 passed, 9 failed, 16 ignored**; all nine failures are existing native restore tests where this environment's Cloud Files API returns `HRESULT 0x80070001`. The baseline was not rerun, so the root cause and whether the baseline fails identically remain unconfirmed. The restore security check remains intact. The focused Studium rerun reported **41 passed, 1 ignored**; the opt-in live test was run separately. The full mocked-IPC UI suite passed with **370 passed, 12 skipped**, using a 60-second per-test timeout.

An explicitly run ignored live test used the production client, the OS root store (before the seat origin was pinned to Sectigo Root R46; the pinned client is unverified on Windows), hostname verification and TLS 1.2 minimum: the common page parsed all seven rooms, and room `5` detail parsed total 102, used 92, remaining 10 with source time `2026-11-09T11:04:57Z`. No school HTML was persisted.

A Windows debug validation app built with `--no-bundle`. Actual UI checks covered default-off settings and no implicit job. One manual cycle returned both rooms `5` and `6` as `ready`: total/used/remaining 102/90/12 at `2026-11-09T11:08:27Z` and 311/55/256 at `2026-11-09T11:08:28Z`. Changing the displayed room during and after that cycle kept `job_id=1`. Closing without disabling and relaunching the same profile retained targets `5,6` and display room `6`, while `job_id` became null and every result was `not_checked`. Turning the extension off and on also retained preferences and started no job. Both validation processes closed normally; no school HTML was persisted. These are separate from installer/release and macOS evidence. The private original returned 404 and the browser review surface was unavailable, so no Python differential comparison ran.

## Restore boundaries

Restore works only on a local fixed NTFS app-data directory. The Cloud Files check rejects registered sync roots and fails closed when it cannot confirm status, but it cannot detect every third-party sync tool. Restore moves files with `MoveFileExW` and stops on ambiguous recovery artifacts. Windows has no directory fsync, so restore survives process interruption, not power loss.
