# Local AI compatibility and evaluation

**v0.3.0 verification is in progress, not release-ready.** The pinned Qwen3.5 **2B and 4B Q4_K_M artifacts have produced useful validated Korean analysis on macOS Apple Silicon with Metal**, one synthetic scholarship sample each. They are now marked `verified` in the catalog with that explicit platform/backend qualification. **9B remains experimental and has not been downloaded or run.** CPU, CUDA and Windows/Linux Qwen3.5 inference remain unverified. Installation integrity, runtime generation, useful output and native end-to-end behavior are separate gates.

## Runtime and text-only loading

The worker pins mistral.rs **v0.9.1**, release commit [`11f255058573e085afe0a149f8a19cbd39aa1200`](https://github.com/EricLBuehler/mistral.rs/tree/11f255058573e085afe0a149f8a19cbd39aa1200). The inspected 0.8.1 and v0.9.0 GGUF routing lacked `qwen35`; this release routes it through native GGUF text loading. Its [`gguf.rs`](https://github.com/EricLBuehler/mistral.rs/blob/11f255058573e085afe0a149f8a19cbd39aa1200/mistralrs-core/src/pipeline/gguf.rs) explicitly excludes `qwen35` from projector-required architectures. This selected text path needs no vision projector. Local loading uses embedded tokenizer/chat-template metadata without a remote tokenizer ID or authentication token. Actual artifact compatibility still requires loading and generation.

Desktop scripts prefer Metal on Apple Silicon only when the existing `xcrun metal --version` probe succeeds; otherwise they build CPU. `RHYTHM_AI_BACKEND=cpu`, `metal` or `cuda` explicitly overrides this selection. These options require existing compatible developer tooling and do not install toolchains or download weights. The worker reports compiled and loaded backend separately and attempts one clean CPU-forced load after an accelerated-load error. Build success or device selection is not a measurement of GPU utilization. Catalog `verified` status does not make the untested CPU/CUDA/platform combinations verified.

## Pinned artifacts

`src-tauri/src/intelligence/catalog.json` is the executable manifest. All candidates are Unsloth GGUF conversions of the corresponding official Qwen3.5 model, with Apache-2.0 license metadata. Manifest checksums come from pinned repository LFS metadata, not the transfer response. These identities must change explicitly if a different artifact is tested. Ordinary one-click installation rejects entries not marked `verified`; 2B/4B are available with their stated qualification, while 9B remains blocked.

| Model | Repository / exact file | Repository revision | Bytes | Quantization | Current evidence |
| --- | --- | --- | ---: | --- | --- |
| Qwen3.5 2B | [`unsloth/Qwen3.5-2B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-2B-GGUF) / `Qwen3.5-2B-Q4_K_M.gguf` | `f6d5376be1edb4d416d56da11e5397a961aca8ae` | 1,280,835,840 | Q4_K_M | Verified: one useful synthetic scholarship analysis, macOS arm64 / Metal only |
| Qwen3.5 4B | [`unsloth/Qwen3.5-4B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF) / `Qwen3.5-4B-Q4_K_M.gguf` | `e87f176479d0855a907a41277aca2f8ee7a09523` | 2,740,937,888 | Q4_K_M | Verified: one useful synthetic scholarship analysis, macOS arm64 / Metal only; initial CPU load failed under memory pressure |
| Qwen3.5 9B | [`unsloth/Qwen3.5-9B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-9B-GGUF) / `Qwen3.5-9B-Q4_K_M.gguf` | `3885219b6810b007914f3a7950a8d1b469d598a5` | 5,680,522,464 | Q4_K_M | Experimental; not downloaded or tested |

SHA-256, in table order:

```text
2B aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223
4B 00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4
9B 03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8
```

Catalog memory values are conservative **estimates** of 4/7/12 GiB, not measured peaks or promises of available capacity. Physical RAM alone does not establish current loader capacity. The 4B option has useful output evidence on the tested Metal setup, but a recommendation remains conditional on backend, current resources and the user's latency tolerance; one sample does not establish superior general quality.

## Actual observations

Environment: **Apple M1 Pro, 16 GiB physical RAM, macOS 27.0.1 arm64**, debug worker, 1,024 maximum output tokens. Both artifacts were installed through the production downloader in isolated test storage and passed size/SHA-256 verification. No production archive was used.

The real `Controller → Worker → validation → Store` path completed the unchanged synthetic `scholarship` fixture for each model with prompt version **2**, result schema **1**. Each final sample passed on its first attempt, without repair. Reports were written to `/tmp/rhythm-evaluation-2b-grounded.json` and `/tmp/rhythm-evaluation-4b-grounded.json`; these are local run artifacts, not required checkout files.

| Model / backend | Load wall time | Complete analysis duration | Worker RSS after load | Worker RSS after analysis |
| --- | ---: | ---: | ---: | ---: |
| Qwen3.5 2B / Metal | 8.121 s | 83.528 s | 2,982,784 KiB | 3,023,456 KiB |
| Qwen3.5 4B / Metal | 12.820 s | 171.156 s | 5,341,680 KiB | 2,030,864 KiB |

These are individual **debug-run observations**, not release performance, isolated warm generation latency, peak/total memory or tokens/second. Complete analysis duration includes production model integrity verification. RSS samples are taken at different points and must not be treated as peaks or used to infer that 4B generally uses less memory. The report's `backend_note` is an explicit harness annotation; it must match the actual worker build/device, not substitute for backend verification. The tested executable was the separately preserved Metal worker.

Observed output:

- **2B:** correct `장학` category and `2026-04-30` deadline. Its summary retained undergraduate status, 12-credit/GPA 3.0 requirements, the 17:00 deadline, email submission, required application/income documents, May 8 results and graduate/on-leave exclusions. Only the deadline became a structured fact. Minor Korean spacing remained (`17 시`).
- **4B:** correct `장학` category, deadline `2026-04-30`, distinct result-announcement date `2026-05-08`, and five grounded facts covering deadline, eligibility, documents, application method and results. Its useful Korean summary retained the major requirements and exclusions. All facts referenced the same exact 205-character source passage, so evidence granularity was coarse.
- Both reports recorded correct category/deadline, no missing expected summary terms and required human review. No invented requirement was observed in these samples; this is not a general hallucination-rate estimate.
- Both established persisted validated output, cache reuse without redundant inference, isolation from a newly created document revision, preserved historical results and a cached read after database reopen with the worker stopped. Each had `stored_history_count: 1`, `reopened_cache_reads_with_worker_stopped: 1` and `current_revision_result_count: 0` after creating the new unanalyzed revision.

Earlier failures remain relevant:

- Initial **4B CPU loading failed** before generation. The loader reported approximately `cpu (avail: 1713MB)` / `1710MB`, exceeding available capacity by `246MB`. This was memory admission, not proof of an unsupported architecture. It produced no CPU summary or generation-throughput result.
- Earlier generation asked the model to reproduce exact evidence quotations. Tokenizer spacing and rewritten quotation prefixes caused legitimate rejection. Prompt-v2 instead selects source-owned passage IDs. A separate Korean date-followed-by-time boundary defect was fixed and regression-tested; invalid evidence is still rejected.
- Apple's Metal Toolchain was installed with explicit approval. Existing Qwen3-0.6B-Q8_0 CPU/Metal generation remains baseline lifecycle evidence; its refusal-like output is not the useful-summary evidence above. A real unavailable-GPU-to-CPU fallback has not been forced.

The two backend reports explicitly have `native_ui_verified: false`; constructing no HTTP client is not a network-denial test. Separate native checks below establish targeted UI/offline behavior. CUDA, Windows/Linux native inference and representative collected-notice quality remain unverified. Linux distribution is still blocked by the existing glib advisory.

A second 2B/Metal run used `academic`, `multiple_dates` and `ambiguous` with the same prompt/configuration. All three completed and passed category, normalized-deadline, expected-summary-term and partial-disclosure screening, with human review still required. Complete analysis durations were **77.997 / 93.078 / 73.192 seconds** respectively. This brings real 2B screening to four of the ten synthetic cases; 4B remains one case. It is not a statistically meaningful acceptance/hallucination rate. The 9B candidate was not downloaded or run: its estimated 12 GiB runtime requirement left insufficient safe headroom on the 16-GiB development host.

## Grounding and repair contract

`intelligence/analysis.rs` uses **prompt version 2** while retaining the stored result schema **1**. The prompt supplies exact contiguous source passages and asks generated facts for an integer `evidence_id`, not a recreated quote. The backend resolves that ID to the archived substring before ordinary strict schema/evidence validation. The stored/UI fact still contains the actual `evidence` text; unsupported IDs, duplicate evidence forms, invalid fields/categories and unsupported evidence fail validation.

For `deadline` and `event_date`, the backend derives a normalized date only from a uniquely identifiable explicit date in the extracted value that is also present in the resolved source passage. It does not trust the model's proposed ISO field or infer missing years. Multiple or missing explicit dates stay null; non-date facts do not acquire dates from incidental passage text. Qualitative uncertainty is not a calibrated probability. Exact source occurrence still does **not** prove that every paraphrase is semantically entailed.

A schema/grounding error permits **one bounded repair attempt** with validation feedback and the same bounded source/output limit. A second invalid response fails the job without replacing earlier good results. Runtime errors do not cause an unbounded retry loop. Prompt/cache identity includes the new prompt and repair configuration; prior results are preserved but do not satisfy a prompt-v2 cache lookup. A recovered pending attempt with an obsolete prompt/schema/configuration identity fails before hashing/inference rather than recording current output under old provenance. Both repair attempts remain cancellable by stopping/reaping the worker.

## Reproducible evaluation

`intelligence/evaluation.json` contains ten **synthetic Korean notices**, not collected university announcements: academic, scholarship, internship, competition, research, multiple dates, no deadline, long input, complex eligibility and ambiguous information. Cases define expected category/deadline, useful summary terms and human-review notes. The long case places important information beyond the bounded prefix, so honest partial disclosure is part of the expected behavior. Scholarship has successful real-model evidence for both 2B and 4B; academic, multiple-date and ambiguous cases additionally passed on 2B.

`intelligence/evaluation.rs::actual_qwen35_analysis_evaluation` uses the real worker, Store and analysis controller with an isolated database. It logs attempt times, outcomes, result JSON and approximate worker RSS where available. Screening checks are not a complete semantic-quality verdict; representative official archived notices still require human review.

Normal CI is network-free and must not download weights. After explicit approval for the artifact/storage budget:

```sh
# Isolated managed installation; this opt-in test also permits experimental candidates.
RHYTHM_MODEL_INSTALL_ID=qwen35-2b \
RHYTHM_MODEL_INSTALL_DIR=/tmp/rhythm-model-validation \
cargo test --locked -p rhythm real_curated_model_install -- --ignored --nocapture

# Build and preserve the exact backend executable before Tauri can restage a sidecar.
cargo build --locked -p rhythm-ai-worker --features metal
cp target/debug/rhythm-ai-worker /tmp/rhythm-metal-worker
RHYTHM_WORKER_TEST_BIN=/tmp/rhythm-metal-worker \
RHYTHM_MODEL_EVAL_ROOT=/tmp/rhythm-model-validation \
RHYTHM_MODEL_EVAL_ID=qwen35-2b \
RHYTHM_AI_EVAL_BACKEND=metal \
RHYTHM_AI_EVAL_CASES=scholarship RHYTHM_AI_EVAL_LIMIT=1 \
RHYTHM_AI_EVAL_REPORT=/tmp/rhythm-evaluation-2b-metal.json \
cargo test --locked -p rhythm actual_qwen35_analysis_evaluation -- --ignored --nocapture
```

Use a dedicated temporary executable/directory and a sufficiently long timeout; debug analyses can take minutes. Substitute the corresponding 4B values to repeat that sample. CPU builds omit `--features metal` and must use a truthful `cpu` annotation; a CPU build does not reproduce the verified Metal result.

`RHYTHM_AI_EVAL_LIMIT` defaults to two samples (maximum ten); `RHYTHM_AI_EVAL_CASES` selects comma-separated case IDs. `RHYTHM_AI_EVAL_RECOVERY=1` additionally exercises cancellation, deliberate test-worker termination and successful recovery; those actual-model branches have not yet been established by the successful reports. `RHYTHM_AI_EVAL_TRACE=/tmp/rhythm-generation.log` enables test-only raw-generation tracing and may contain source-derived text; production builds do not retain that trace.

Tauri's build step can overwrite `target/debug/rhythm-ai-worker` with the previously staged sidecar. A Fresh Cargo build message alone does not guarantee that path still contains the desired backend/protocol implementation after another app build. Preserve the worker separately for evaluation and explicitly stage the correct worker before native builds.

## Remaining evidence

Broaden exact-artifact evaluation across representative categories and collected notices, measuring structured-output acceptance/failure and repair frequency, classification/date/evidence correctness, important-fact preservation and unsupported speculation. Separate warm generation, model loading and complete pipeline duration; do not extrapolate from the one-sample timings. Keep untested sizes/backends unverified.

### Separate native macOS verification

A debug `.app` with the isolated identifier `io.github.hiyabye.rhythm.v03validation` upgraded a synthetic populated schema-2 archive to schema 3. The saved notice, dark theme, performance preference and sentinel interests survived. Production app data was untouched.

Native accessibility actions exercised the actual Svelte/Tauri/SQLite/worker path, with no mocked IPC or committed automation hooks:

- 2B download, visible progress, cancellation and resume, size/hash validation, selection, loading and actual short-inference setup completion.
- Reader-generated Korean summary, `장학` classification and grounded `2026-04-30` deadline; first complete analysis **82.319 seconds**. List AI preview and immediate cached reader reopening created no additional job.
- Restart under `sandbox-exec` with `(deny network*)`; a separate network probe was denied. Cached reading and actual regeneration succeeded, **87.721 seconds**, without university refetch or remote inference.
- Running-request cancellation preserved earlier results. A sampled worker stack confirmed Qwen forward execution before deliberate worker termination; the UI reported failure while keeping the article/results accessible. A second sampled forward pass was cancelled through the reader, and its worker was reaped. A subsequent analysis successfully restarted/reloaded the worker under the same network denial, **88.037 seconds**.
- Eco idle maintenance replaced the loaded worker with an empty worker (observed RSS about 12 MiB), releasing the model. Closing during command-owned model-load/integrity checking exited promptly and removed the worker. Final restart retained selected/installed model, completed setup, archive/read/save state, updated Eco preference and four successful result-history records. SQLite integrity was `ok`, with no foreign-key violations.
- A fresh build through the desktop helper automatically selected Metal; its final isolated native restart showed compiled Metal accurately, read the cached analysis offline and exited with no orphan worker. A separately staged persisted pending job prevented managed-model deletion; the failed deletion retained the selected installed model. Cancelling that pending job through Settings succeeded without inference.

These are targeted checks with a **synthetic archived notice**, not full accessibility conformance, all source formats, every lifecycle interleaving or collected-notice quality. Shutdown during active kernel loading/setup generation was not separately timed; command reset/exit regressions use controlled local child processes. No app/worker inference HTTP server was introduced.

Final automated checks include 129 passing locked workspace Rust tests (six opt-in tests ignored), 62 mocked-IPC Chromium/WebKit checks, five POSIX desktop orchestration checks, zero Svelte diagnostics, Rustfmt, strict workspace Clippy and a debug macOS app build. Metal and lifecycle-only worker variants separately passed ten tests each. Release-optimized installers/signing and Windows/Linux/CUDA runtime remain unverified. See [roadmap](ROADMAP.md#v030--local-intelligence) and [architecture](ARCHITECTURE.md) for implementation boundaries and remaining limits.
