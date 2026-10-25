# v0.3.0 implementation handoff — historical pause checkpoint

> Resumed at the user's request. The implementation subsequently passed integrated checks and targeted native 2B/Metal offline/cancellation/crash-recovery verification. Current results and limitations are in [AI evaluation](AI_EVALUATION.md) and [roadmap](ROADMAP.md). The remainder records the earlier pause and is not current status or a new instruction to pause.

Paused checkpoint, 2026-10-25. This is working implementation, not a completed/released v0.3.0. No commits or pushes made. Starting commit: `9073d2c` (`fix: stabilize Rhythm v0.2.1 archive foundation`). The initial working tree was clean; current uncommitted changes belong to this task.

## Approved decisions and resource permissions

User approved via ask_user:

- Keep two crates, collectors unchanged, schema 3 with separate model inventory/preferences, persistent jobs/results, single inference queue, stop/reap/restart cancellation, targeted mistral.rs upgrade.
- Download up to **12 GB total model weights**, attempting 2B/4B/9B only if resources safely permit. Downloaded so far approximately **4.02 GB** (2B + 4B), no 9B.
- Install Apple's missing Metal Toolchain. `xcodebuild -downloadComponent MetalToolchain` succeeded; `xcrun metal --version` now works (Apple metal 32023.921, toolchain 27A266a).
- No production archive modifications, remote pushes, publishing or deployment.

Machine: Apple M1 Pro, arm64, macOS 27.0.1, 16 GiB RAM. Last observed free disk **15 GiB**. Do not exhaust it with 9B/release builds. Cargo is installed but not on default tool PATH: `export PATH="$HOME/.cargo/bin:$PATH"`.

## Implemented, still uncommitted

- `003_intelligence.sql`: schema 3; `ai_models`, `ai_preferences`, `ai_jobs`, `ai_results`. Released migrations unchanged. Upgrade/recovery/preservation/cache tests.
- `storage/intelligence.rs`: inventory/preferences, semantic cache, transactional jobs/results, authoritative active-job queries (not just last 100 history).
- `intelligence/models.rs` + `catalog.json`: curated pinned artifacts, constrained HTTPS/public DNS pinning/CDN redirects, progress/cancel/strong ETag resume/size+SHA-256/disk reserve/atomic finalization, external reference registration (qwen3/qwen35 preflight), confirmed safe deletion.
- Worker: pinned **mistral.rs v0.9.1 Git release commit `11f255058573e085afe0a149f8a19cbd39aa1200`**. Registry still only had 0.8.1. Both 0.8.1 and v0.9.0 lacked qwen35 GGUF routing; v0.9.1 supports text-only qwen35 without projector. Additive protocol-v1 load/unload, bounded deterministic inference, separate system instruction, single model, metrics, file metadata identity, truthful loaded/build backend fields, one CPU fallback attempt after accelerated loading errors.
- Main Worker client: infer/load/unload/restart/stop, deadlines, hard cancellation/reaping, serial reuse, typed protocol/status tests.
- `analysis.rs`: persistent single queue, artifact integrity, local revision input only, Korean summaries/taxonomy/facts, semantic cache, cancellation, strict structured validation. **PROMPT_VERSION=2 / schema=1**. Source-owned `evidence_id` passages prevent fabricated quotation bytes; backend derives normalized explicit dates; one bounded repair attempt. Bounded prefix with honest partial counts; no binary attachments.
- `commands.rs`: Tauri AI commands, nonblocking snapshot, selection/loading/removal/preferences/setup real inference, analysis/results. AI mutation gate; unload before external re-registration to prevent old model/new hash mislabeling; explicit load/setup inference selects against exit flag. Active jobs merged with bounded terminal history.
- `maintenance.rs`: app-open focused-window opt-in recent processing only after startup, no historical flood, session caps, Eco explicit-only; idle unload 30/120/600 sec. Not a tray/background scheduler.
- Frontend: `AiSettings.svelte`, `AnalysisPanel.svelte`, `ai.css`, App/reader/list/Today integration, first-run nonblocking banner, complete controls and empty/error/loading/cache/partial/history states. Generated text safely escaped.
- Playwright fixture and 12 new cases. Metadata set to 0.3.0. Documentation updated provisionally.

## Actual model evidence

Pinned models in `/tmp/rhythm-model-validation/models/` (isolated test data):

| Model | Repository revision | File | Bytes | SHA-256 |
| --- | --- | --- | ---: | --- |
| 2B | unsloth/Qwen3.5-2B-GGUF @ f6d5376be1edb4d416d56da11e5397a961aca8ae | Qwen3.5-2B-Q4_K_M.gguf | 1280835840 | aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223 |
| 4B | unsloth/Qwen3.5-4B-GGUF @ e87f176479d0855a907a41277aca2f8ee7a09523 | Qwen3.5-4B-Q4_K_M.gguf | 2740937888 | 00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4 |
| 9B | unsloth/Qwen3.5-9B-GGUF @ 3885219b6810b007914f3a7950a8d1b469d598a5 | Qwen3.5-9B-Q4_K_M.gguf | 5680522464 | 03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8 |

All Apache-2.0 provenance. 2B/4B downloaded through production downloader's opt-in test and independently checked against committed manifest hashes. 9B not downloaded/tested. Estimates in catalog are not measured memory.

**Useful real inference succeeded for 2B and 4B Metal**, one unchanged synthetic Korean scholarship case each through actual Controller → Worker → validation → SQLite. Correct category/deadline, grounded facts, useful Korean summary, cache reuse, new-revision isolation, historical preservation and cached read after DB reopen with worker stopped. Not native UI or OS-enforced offline proof.

| Model | Load wall | Analysis duration (includes model hashing) | Post-load worker RSS observation |
| --- | ---: | ---: | ---: |
| 2B Metal | 8.121 s | 83.528 s | 2,982,784 KiB |
| 4B Metal | 12.820 s | 171.156 s | 5,341,680 KiB |

Debug observations, not release throughput/peak memory. Both final samples succeeded on first attempt. 2B structured only deadline; 4B had five grounded facts. Korean spacing imperfect. Source occurrence checks do not establish full semantic entailment.

Evidence files (temporary, not Git):

- `/tmp/rhythm-evaluation-2b-grounded.json`, `/tmp/rhythm-evaluation-4b-grounded.json`
- `/tmp/rhythm-2b-trace-grounded.log`, `/tmp/rhythm-4b-trace-grounded.log`
- `/tmp/rhythm-4b-install.log`, `/tmp/rhythm-2b-install.log`
- `/tmp/rhythm-evaluation-4b-cpu.json`: initial CPU load failed, available capacity ~1710 MiB, short by246 MiB. CPU compatibility still unverified for Qwen3.5. Do not confuse this with architecture failure.
- Initial prompt-v1 generation failed exact evidence checks; trace and date-boundary regression led to prompt-v2 source evidence references and deterministic date handling. Invalid evidence remains rejected.

**Catalog just promoted 2B/4B to `verified` with explicit reason that evidence is macOS Apple Silicon Metal only; 9B remains experimental.** This late change has not had a final full integrated test rerun. Documentation still says all models experimental in places and needs synchronization.

## Verification already actually run (checkpoint-specific)

- Storage: 28 passing storage tests at first checkpoint (1 existing benchmark ignored); after active-job fix 7 intelligence storage tests passed. Schema1 and populated-schema2 upgrade, migration rollback, archive/preferences preservation, FK/SQLite/FTS/cache/dedup/recovery covered.
- Models: 10 deterministic tests passed + real multi-GB install intentionally ignored ordinarily. Both actual 2B/4B install probes separately passed. Deletion regression was verified to fail against old last100-jobs guard.
- Analysis: final 12 deterministic tests passed; bounded repair test, process cancellation/cache/recovery, evidence/date/prompt-injection boundary, long/partial input. Korean date-followed-by-time regression verified against old condition.
- Worker: CPU, Metal, lifecycle-only each 7 unit +3 process tests passed (actual model ignored ordinarily); Clippy all variants passed. Real Qwen3-0.6B CPU baseline preserved; Metal baseline generated actual tokens but refusal-like greeting (not useful quality evidence).
- Main Worker client: 7 tests passed with actual worker configured at earlier checkpoint.
- Frontend: `npm run check` zero warnings/errors, production build passed; **62 Playwright checks** (31 Chromium +31 WebKit) passed before later backend-only fixes. These mock IPC, not native backend tests. UI completion race regression verified against removed fix.
- Strict application Clippy passed in analysis agent's final checkpoint; full workspace/final frontend suites still need rerun after all integration.
- Debug macOS `.app` built via `npm exec tauri -- build --debug --bundles app --config '{"identifier":"io.github.hiyabye.rhythm.v03validation"}'`; log `/tmp/rhythm-native-build.log`. **Not launched/tested yet.** This build temporarily included native test hooks, removed from source before pause. Rebuild before using a final artifact.
- No final full workspace suite, release build, native offline flow, native cancellation/crash recovery, Windows/Linux runtime, or full ten-case real-model evaluation claim.

## Native verification preparation (not executed)

A separate **synthetic schema-2** application fixture exists at:

`~/Library/Application Support/io.github.hiyabye.rhythm.v03validation/rhythm.sqlite3`

It has one scholarship document/revision with saved state, dark/performance preferences and sentinel interests `업그레이드 보존 검증`, notice extension disabled. It was created from released migrations using Python SQLite and has **not been opened by the native app yet**. It is deliberately separate from production app data. Native test should prove migration and these sentinel values survive.

There are **no active task-owned cargo/download/evaluation/native app processes** at pause (checked process table). No production app archive was changed. Temporary test-only `native_check_record` command and `RHYTHM_NATIVE_SCRIPT` page eval hook were removed from `main.rs` before pause; no arbitrary script hook should be reintroduced in committed source.

## Important tooling gotcha

**Tauri's build step copies the staged sidecar over `target/debug/rhythm-ai-worker`.** An old staged worker caused misleading `알 수 없는 요청입니다.` load errors even after `cargo build` said Fresh.

- Current known Metal worker copy: `/tmp/rhythm-metal-worker` (used for successful evaluations).
- Current staged `src-tauri/binaries/rhythm-ai-worker-aarch64-apple-darwin` was updated to Metal.
- Before app checks/builds, build the desired worker and stage that exact new binary. For evaluations use a separate copied worker path so Tauri cannot overwrite it.
- `scripts/desktop.mjs` STILL builds/stages CPU default; optional Metal packaging selection is not implemented yet. Decide the minimal integration so users get validated acceleration where appropriate without forcing unverified tooling on all platforms. GPU fallback code exists.

## Next steps when user resumes

1. Read this handoff and current diff/status; do not restart from scratch or repeat approved questions.
2. Run quick fmt/check/Clippy/integrated workspace tests with an explicitly freshly staged worker. Ensure catalog status tests don't assume all experimental.
3. Review late integration fixes: all conflicting admission paths use `ai_gate`; explicit load/setup exit cancellation, registration unload, active-job snapshots. Add command-level shutdown/reg-registration/race tests if practical. Worker status currently does not surface additive loaded_backend to Svelte; consider accurate settings reporting. Hardware disk value cached at startup may be stale (installer itself checks live disk).
4. Perform isolated actual native UI download → load → setup → reader analyze → cache → restart, preserving schema2 fixture. Use2B to limit time/disk; a fresh native UI download adds1.28GB within approved12GB budget. Native instrumentation, if used, must be temporary and removed before final artifacts/commit.
5. OS-enforced offline read/regenerate, actual cancel, kill-worker recovery, normal quit cleanup. Do not label browser-offline mock test as native proof.
6. Broaden real model evaluation beyond one synthetic scholarship case if feasible. Ten reproducible fixtures exist in evaluation.json; real collected notice quality still untested. 9B might be unsafe/unhelpful under memory/disk pressure: keep unverified if not run; no need to consume budget just because authorized.
7. Sync docs with actual 2B/4B Metal successes and PROMPT_VERSION2/evidence references/one repair attempt. Existing docs were written before final successful inference and catalog promotion. Keep limitations and Linux glib distribution gate.
8. Complete metadata consistency, full diff/security review, relevant tests, final build. Remove temporary verification files/hooks from source, not valuable unrelated user files.
9. Commit coherent completed feature with Conventional Commits only after verified; no push/publish. If requirements remain unmet, explicitly report v0.3 release gates rather than marking done.

Useful explicit evaluation command:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
RHYTHM_WORKER_TEST_BIN=/tmp/rhythm-metal-worker \
RHYTHM_MODEL_EVAL_ROOT=/tmp/rhythm-model-validation \
RHYTHM_MODEL_EVAL_ID=qwen35-2b \
RHYTHM_AI_EVAL_BACKEND=metal \
RHYTHM_AI_EVAL_CASES=scholarship RHYTHM_AI_EVAL_LIMIT=1 \
RHYTHM_AI_EVAL_TRACE=/tmp/rhythm-next-trace.log \
RHYTHM_AI_EVAL_REPORT=/tmp/rhythm-next-evaluation.json \
cargo test --locked -p rhythm actual_qwen35_analysis_evaluation -- --ignored --nocapture
```

Use sufficiently long timeouts; debug model analyses take minutes. Trace is test-only and may contain source-derived generated text. No cloud inference/network tool capabilities are exposed to models.
