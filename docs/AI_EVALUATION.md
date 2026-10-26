# Local AI compatibility and evaluation

**v0.3.1 hardening does not establish distribution readiness or reliable semantic extraction.** The pinned Qwen3.5 **2B and 4B Q4_K_M artifacts have produced useful Korean analysis on macOS Apple Silicon with Metal**. Catalog `verified` means that qualified artifact/runtime evidence, not universal factual correctness. Broader v0.3.1 testing below exposes errors that strict literal grounding cannot catch. **9B remains experimental and has not been downloaded or run.** CPU, CUDA and Windows/Linux Qwen3.5 inference remain unverified. Installation integrity, runtime generation, source grounding, semantic usefulness and native behavior are separate gates.

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

## Historical v0.3.0 observations

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

`intelligence/analysis.rs` now uses **prompt version 3** while retaining stored result schema **1**. Explicit `{id, text}` source objects avoid implicit numbering. Generated facts must contain exactly `kind`, `label`, `value` and integer `evidence_id`; the backend supplies the exact archived evidence, date and per-fact uncertainty. Legacy direct-evidence objects cannot bypass generation validation, while old stored schema-1 results remain readable.

The selected passages fit an 18-KiB encoded source budget inside the 24-KiB prompt. Over-budget input selects bounded head, tail and important passages, suppressing exact duplicate passage text; complete input keeps every occurrence. Table rows use independent evidence units and numeric date punctuation is not a sentence boundary. Original byte ranges and source order are preserved. Omitted text still makes the result partial: this is neither chunk-and-merge nor complete-document understanding. Selector and output-affecting prompt rules participate in cache identity.

For `deadline` and `event_date`, normalization requires a uniquely identifiable explicit date in the extracted value and its resolved source passage. Matching preserves digit boundaries; it cannot join `2026.4.3 0명` into April 30. Claimed clocks must have literal support tied to the claimed date; mismatched or unsupported AM/PM claims fail. Missing years, relative dates, ambiguous periods and multiple explicit dates remain unnormalized. Even a linguistically plausible year expansion in an abbreviated range is not accepted as a fully explicit source date. Repair feedback asks the model to retain the incomplete original notation instead. Non-date facts cannot acquire incidental dates.

Qualitative uncertainty is not calibrated confidence. These checks do **not** validate the meaning of a label, determine whether a deadline is still active, or prove that a paraphrase preserves exclusions. A correctly quoted cancelled date can still be incorrectly classified as a current deadline. Prompts request preservation of conditions, negation, temporal purpose and explicit corrections, but that instruction is not a semantic guarantee.

A schema/grounding error permits **one bounded repair attempt** with validation feedback and the same bounded source/output limit. A second invalid response fails the job without replacing earlier good results. Runtime errors do not cause an unbounded retry loop. Prompt/cache identity includes prompt, selector, output-affecting rules and repair configuration; prior results are preserved but do not satisfy a newer semantic key. A recovered pending attempt with an obsolete prompt/schema/configuration identity fails before hashing/inference rather than recording current output under old provenance. Both repair attempts remain cancellable by stopping/reaping the worker.

## Reproducible evaluation

`intelligence/evaluation.json` contains **18 synthetic Korean notices**, not collected university announcements: academic, scholarship, internship, competition, research, multiple dates, no deadline, long input, complex eligibility, ambiguous information, event-only, formatted table, incomplete source, yearless deadline, relative date, distinct time purposes, long-tail correction and eligibility exclusions. Cases define expected category, the complete set of normalized deadline dates, useful summary terms and human-review notes. Long cases require the tail deadline/correction and honest partial disclosure; they no longer expect a prefix-only omission.

`intelligence/evaluation.rs::actual_qwen35_analysis_evaluation` uses the real worker, Store and analysis controller with an isolated database. It records per-attempt metrics, stage timings, result JSON and approximate worker RSS. Screening rejects missing **and extra** normalized deadlines, checks category/summary terms/partial disclosure and independently rechecks stored typed shape and literal source validation. These checks do not determine semantic correctness; term matching can also flag a valid paraphrase. Human review must inspect dates without normalized years, fact labels, exclusions, cancellation, required documents and submission method.

Normal CI is network-free and must not download weights. After explicit approval for the artifact/storage budget:

```sh
# Isolated managed installation; this opt-in test also permits experimental candidates.
RHYTHM_MODEL_INSTALL_ID=qwen35-2b \
RHYTHM_MODEL_INSTALL_DIR=/tmp/rhythm-model-validation \
cargo test --locked -p rhythm real_curated_model_install -- --ignored --nocapture

# Build and preserve the exact backend executable before Tauri can restage a sidecar.
cargo build --locked --release -p rhythm-ai-worker --features metal
cp target/release/rhythm-ai-worker /tmp/rhythm-metal-worker
RHYTHM_WORKER_TEST_BIN=/tmp/rhythm-metal-worker \
RHYTHM_MODEL_EVAL_ROOT=/tmp/rhythm-model-validation \
RHYTHM_MODEL_EVAL_ID=qwen35-2b \
RHYTHM_AI_EVAL_BACKEND=metal \
RHYTHM_AI_EVAL_CASES=scholarship RHYTHM_AI_EVAL_LIMIT=1 \
RHYTHM_AI_EVAL_REPORT=/tmp/rhythm-evaluation-2b-metal.json \
cargo test --locked --release -p rhythm actual_qwen35_analysis_evaluation -- --ignored --nocapture
```

Use a dedicated temporary executable/directory and a sufficiently long timeout; debug analyses can take minutes. Substitute the corresponding 4B values to repeat that sample. CPU builds omit `--features metal` and must use a truthful `cpu` annotation; a CPU build does not reproduce the verified Metal result.

`RHYTHM_AI_EVAL_LIMIT` defaults to two samples (maximum 18); `RHYTHM_AI_EVAL_CASES` selects comma-separated case IDs. The bounded controller timeout is 1,500 seconds, allowing integrity/load and at most two 600-second inference attempts. `RHYTHM_AI_EVAL_RECOVERY=1` additionally exercises cancellation, deliberate test-worker termination and recovery. It now observes the parent's inference IPC phase rather than treating mutex contention as proof of inference; this is still not proof of an active model kernel. Historical native crash/cancel evidence is separate from this optional harness branch. `RHYTHM_AI_EVAL_TRACE=/tmp/rhythm-generation.log` enables test-only raw-generation tracing and may contain source-derived text; production builds do not retain that trace.

Tauri's build step can overwrite `target/debug/rhythm-ai-worker` with the previously staged sidecar. A Fresh Cargo build message alone does not guarantee that path still contains the desired backend/protocol implementation after another app build. Preserve the worker separately for evaluation and explicitly stage the correct worker before native builds.

### Release latency benchmark and local public-archive evaluation

`actual_analysis_latency_benchmark` times a cold model, forced warm regeneration and immediate cached lookup on the same scholarship input. Use the same release parent/worker/artifact/backend and supply `RHYTHM_AI_BENCH_LABEL` plus `RHYTHM_AI_EVAL_REPORT`. Worker startup is separate. Cold means an empty worker model, not a cold filesystem cache. Per-request loading/reuse, token counts and upstream timings remain distinct from parent-stage and end-to-end wall times; missing older-worker metrics remain unknown.

`archived_evaluation.rs::actual_archived_notice_evaluation` imports explicitly selected revisions from an **existing read-only public-notice archive** into a fresh isolated store. It does not open the source through `Store::open`, migrate the original or refetch the university. Set `RHYTHM_AI_EVAL_ARCHIVE` to that SQLite file and `RHYTHM_AI_ARCHIVE_IDS` to at most ten comma-separated document IDs, alongside the worker/model/report variables above. The report retains original representation identity, source title/body, generated results, cache/reopen checks and diagnostics for local review. No HTTP client is created; that alone is not OS-enforced network denial. Raw reports/traces contain source-derived text: keep them local, uncommitted and unuploaded. Original source rights are unchanged.

## v0.3.1 evidence and remaining gates

Completed release-mode prompt-v3/selector-v4 screening covered all **18 synthetic cases and six already-archived public notices per artifact**, followed by the bounded same-input decoding experiment below. The initial unconstrained campaign completed 17/18 synthetic jobs on each model, 3/6 archived jobs on 2B and 6/6 on 4B. Expected-category matches were 10/17 and 17/17 completed synthetic outputs respectively. These are **intermediate prompt settings**, not the final mode's pass rates. Literal-valid results still included omitted table deadlines, an obsolete date retained as a deadline and an excluded population presented as eligible. Later wording/decoding changes must not inherit those scores.

Broader factual-quality, CPU/CUDA/9B/platform and distribution gates remain open. The following comparison isolates full-file hashing rather than attributing later prompt changes to that optimization.

### Isolated full-file hashing comparison

Same host, exact artifacts, Metal release worker and release parent; unchanged prompt **2**, scholarship input, 1,024 output limit and complete-file integrity checks. The only intended change between these bounded runs was the approved macOS-arm64 `sha2/asm` feature. Both known artifact digests matched. Reports: `/tmp/rhythm-v031-baseline-qwen35-{2b,4b}.json` and `/tmp/rhythm-v031-accelerated-qwen35-{2b,4b}.json`.

| Artifact | Cold / warm complete analysis, software hash | Cold / warm, accelerated hash | Integrity stage, software → accelerated | Immediate cached lookup |
| --- | ---: | ---: | ---: | ---: |
| 2B | 15.831 / 14.243 s | 12.716 / 10.721 s | 3.882 / 3.816 → 0.808 / 0.737 s | 208–257 µs |
| 4B | 34.328 / 33.553 s | 27.859 / 25.654 s | 8.299 / 8.502 → 1.635 / 1.762 s | 226–349 µs |

These are one cold and one warm sample per condition, not statistical estimates or all-document speedups. Cache reads perform neither model hashing nor inference. No mtime/hash bypass, shortened digest or persisted integrity cache was introduced. New prompt/selection quality work changes prefill and generated content, so final prompt-v3 timings must be reported separately.

### Bounded schema-decoding experiment

The user approved a coordinated optional worker mode using the already-pinned runtime's JSON-schema constraint support; no dependency, model download or database migration was added. Both conditions used the same release parent/Metal worker, exact artifact, positive primary-purpose prompt wording, selector v4, source and 1,024 output limit. Eight synthetic cases covered scholarship, research, multiple dates, no deadline, complex eligibility, table rows, time purposes and long-tail correction. Three selected public records (IDs 1/4/9 from the existing read-only test archive) specifically exercised earlier failures. This is a small targeted comparison, not a representative benchmark.

| Artifact / mode | Completed synthetic jobs | Expected categories / completed | Complete expected deadline-date set / completed | Completed archived jobs |
| --- | ---: | ---: | ---: | ---: |
| 2B / unconstrained | 8/8 | 3/8 | 6/8 | 0/3 |
| 2B / `analysis_json_v1` | 8/8 | 3/8 | 6/8 | 1/3 |
| 4B / unconstrained | 7/8 | 6/7 | 6/7 | 2/3 |
| 4B / `analysis_json_v1` | 8/8 | 6/8 | 7/8 | 3/3 |

Completed jobs improved **17/22 → 20/22**, without proving that all accepted facts were correct. Two remaining constrained 2B public-notice failures rejected unsupported year expansions in abbreviated dates; strict validation and one repair were not bypassed. Both 2B modes still missed the first table deadline and retained the cancelled deadline. 4B preserved both table deadlines but also retained an obsolete deadline. Classifier errors remained. A `deadline` value containing an entire valid date range can also fail the normalized-end-date screen even when the raw range is faithful; manual review distinguishes this from an invented date.

| Artifact / mode | Cold / warm complete analysis | Cold / warm integrity | Immediate cached lookup |
| --- | ---: | ---: | ---: |
| 2B / unconstrained | 15.527 / 14.052 s | 0.690 / 0.690 s | 215 / 232 µs |
| 2B / constrained | 15.425 / 13.957 s | 0.688 / 0.688 s | 203 / 194 µs |
| 4B / unconstrained | 30.014 / 28.389 s | 1.581 / 1.493 s | 218 / 205 µs |
| 4B / constrained | 29.841 / 28.104 s | 1.505 / 1.496 s | 225 / 210 µs |

These single-case timings are effectively similar, not evidence of a general decoding speedup. Different prompt/content means they cannot be compared as a controlled speedup against prompt-v2 results. Long-tail 2B processing in the intermediate selector-v4 campaign fell from 30.643 to 15.547 seconds alongside exact-repeat suppression and prompt changes; that is a bounded synthetic observation, not a real-world all-document improvement claim.

**Decision (user-approved after reviewing the comparison):** enable capability-negotiated `analysis_json_v1` for analysis because it improved structural completion in this bounded comparison without material observed benchmark overhead. Setup remains ordinary text. The static closed grammar limits fields/types/enums and at most six facts; it does not establish semantic correctness or enforce fact-identity uniqueness. Parent source/date/time/link/duplicate checks, one repair and process cancellation remain unchanged. An older worker is rejected with rebuild guidance before receiving an unknown infer parameter; ordinary text on that same legacy pipe remains usable.

An initial constrained 2B smoke generated a valid object followed by repeated `<|endoftext|>` until 1,024 tokens and failed strict parsing. Adding that grammar-EOS stop sequence **only in analysis mode** produced a completed, non-length-terminated 347-token response; no suffix was trimmed to manufacture validity. Completed A/B outputs subsequently passed strict parsing and literal-source checks on both artifacts. Decoder mode has a distinct cache configuration: same-document/revision/artifact benchmark keys differ, and the Store regression verifies no cross-mode cache hit.

Reproduce using the release commands above with test-only `RHYTHM_AI_EVAL_DECODING=unconstrained` or `analysis_json_v1`; ordinary application builds use the adopted constrained default. Reports record `decoding_mode`. Local reports/traces are `/tmp/rhythm-v031-schema-ab-{2b,4b}-{unconstrained,analysis_json_v1}-{synthetic,archive,benchmark}.json` and matching generation logs. Do not commit/upload source-derived reports. A subsequent deterministic regression reproduced an English-meridiem bug: `5:00 PM` could support a wrongly claimed `오전 5시`. The parent now recognizes bounded AM/PM prefix/suffix clocks (including noon/midnight), rejects mismatched/conflicting markers and avoids interpreting a date's day or a clock's minutes as another hour. This changed only validation, not generation prompts; `clock_rules=meridiem-v2` separates old cache/recovery identities. Regression tests, rather than a new model-language-quality campaign, verify these English forms. The full 18-case campaign has **not** been rerun under this final default, so its broader final semantic acceptance rate remains unknown.

### Targeted v0.3.1 native macOS verification

A debug app built with `io.github.hiyabye.rhythm.v031validation` used a fresh isolated copy of the earlier synthetic validation profile, not production data. Accessibility actions exercised actual native IPC and the installed 2B/Metal worker, without application automation hooks:

- The backend-owned load operation remained visible after leaving Settings and returning; cancellation during loading stopped the worker, cleared the operation and allowed later work.
- Cancelling setup during verification left `setup_completed=0`; this is not a claim of cancellation during an active setup-generation kernel. A subsequent actual short-inference health check persisted completion successfully.
- An OS-network-denied restart (`sandbox-exec`, independent network probe exit 6) displayed the older stored analysis with a prompt/configuration mismatch warning. Regeneration using prompt 3/selector v4 completed in **49,530 ms** in this debug app and replaced the warning with the saved analysis. This is one functional observation, not a release-performance comparison or complete-document quality claim. Later wording-only prompt tuning is evaluated separately by the release harness.
- Network-denied debug bundles with the adopted constrained default warned about previous cache identities and regenerated successfully in **50,596 ms**, then **104,881 ms** with the final versioned clock guard. Both stored their current configuration hashes. These are uncontrolled functional runs, not a speed comparison. Settings reported actual loaded backend **Metal**. The native regeneration's category was still `학사` for this scholarship notice, illustrating that successful constrained/offline completion is not semantic correctness.
- After normal quit, the copied document retained read/saved state, SQLite integrity was `ok` and foreign-key checks were empty. No orphan native worker remained.

Automated checkpoint: **175 passing locked Rust tests** (161 application, ten worker unit, four process; eight opt-in tests ignored), strict workspace Clippy, Rustfmt, zero Svelte diagnostics, **86 mocked-IPC Chromium/WebKit checks**, five POSIX desktop checks and a debug macOS app/DMG build. The added Ubuntu desktop-test CI step is configuration, not evidence of a remote run. Command cancellation regressions cover cooperative verification and controlled IPC processes; a full suite first exposed and then passed after fixing a cancellation-result race. Windows/Linux native behavior, full accessibility and active-kernel setup shutdown remain unverified.

### Historical v0.3.0 native macOS verification

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
