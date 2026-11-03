# Windows validation

What has actually been observed on Windows 11 x64. Build setup is in [Windows](WINDOWS.md).

## Sources of evidence

- **Contributor PC (PR #1 and PR #2), 2026-11-02 to 2026-11-03.** One Windows 11 Home x64 PC (build 26300, Intel Core i5-10300H, 16 GB RAM, NVIDIA GTX 1650 4 GiB, driver 552.44). Most installed-app observations used the port's old identifier `app.rhythm.windows` and builds 0.9.2–0.9.4, before the merge into the shared `io.github.hiyabye.rhythm` codebase. They do not verify the current build.
- **GitHub CI (`windows-latest`).** Every pull request and push to `main` that touches Rust runs Clippy and tests for the workspace on a hosted Windows Server runner, including GPU fallback tests against a fake `llama-server` and a lifecycle test of the staged real builds without model weights. `main` also builds the NSIS installer when packaging inputs change. CI has no NVIDIA GPU, so these tests check supervision and fallback, not inference.

No installed-app run of the merged build, second PC or VM has been tested.

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

These observations used the former AI workers (mistral.rs CPU and llama.cpp Vulkan built into Rhythm's own worker). The bundled `llama-server` that replaced them has not been run on Windows yet.

Each row is one case on the contributor PC, often with other builds running, so durations are not benchmarks. Summary correctness was excluded from the GPU feature's completion criteria at the user's direction; validation still rejects ungrounded output.

| Backend / model | Observed |
| --- | --- |
| CPU, 2B | Pinned file downloaded and SHA-256-verified; real download cancel and HTTP 206 resume matched the pinned hash. Synthetic scholarship notice: schema, grounding and deadline passed, category wrong (`학사` instead of `장학`). Cancellation, worker termination and recovery passed. A collected notice passed structure checks, but its deadline stayed unnormalized and the summary dropped a date qualifier. Worker memory-limit (Job Object) and restart test passed |
| CPU, 4B | Synthetic scholarship case passed quality, recovery and cache checks. Collected notice: structure passed, deadline unnormalized. Peak sampled working set about 8.8 GiB |
| GPU (Vulkan, GTX 1650), 2B / 4B | Both loaded and generated with 25 / 31 offloaded layers; model reuse, worker termination, reload and shutdown passed. Structured output was valid but both missed the expected category. Through the app pipeline, 2B generated twice and grounding validation rejected the output, so no result was stored |

Not verified: other NVIDIA GPUs or drivers, GPU-less fallback in the installed app, VRAM exhaustion, OS-enforced offline operation, and AI from the installed GPU settings UI.

## Restore boundaries

Restore works only on a local fixed NTFS app-data directory. The Cloud Files check rejects registered sync roots and fails closed when it cannot confirm status, but it cannot detect every third-party sync tool. Restore moves files with `MoveFileExW` and stops on ambiguous recovery artifacts. Windows has no directory fsync, so restore survives process interruption, not power loss.
