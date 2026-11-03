# AI evaluation

What local AI has been verified to do, the pinned artifacts, and how to run the opt-in evaluations. The analysis contract (prompt, grounding, repair, cache identity) is in [architecture](ARCHITECTURE.md#analysis-validation-and-cache). Installation integrity, runtime generation, source grounding, semantic usefulness and native behavior are separate claims.

## Runtime

CPU, Apple Metal and Windows Vulkan use the same llama.cpp implementation through `llama-cpp-2`/`llama-cpp-sys-2` pinned to `0.1.160`. The runtime uses local GGUF tokenizer/chat metadata without a remote tokenizer or vision projector. Windows keeps separate CPU and Vulkan executables for fallback; every worker ships with the app at its version. New analyses use runtime policy `llama-unified-v1`; previous results remain readable but cannot satisfy the new cache key.

## Pinned artifacts

`src-tauri/src/intelligence/catalog.json` is the executable manifest: Unsloth GGUF conversions of the official Qwen3.5 models, Apache-2.0, checksums from the pinned repository LFS metadata. One-click installation accepts only `verified` entries.

| Model | Repository / file | Revision | Bytes | Status |
| --- | --- | --- | ---: | --- |
| Qwen3.5 0.8B | [`unsloth/Qwen3.5-0.8B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF) / `Qwen3.5-0.8B-Q4_K_M.gguf` | `6ab461498e2023f6e3c1baea90a8f0fe38ab64d0` | 532,517,120 | Experimental: never downloaded or run |
| Qwen3.5 2B | [`unsloth/Qwen3.5-2B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-2B-GGUF) / `Qwen3.5-2B-Q4_K_M.gguf` | `f6d5376be1edb4d416d56da11e5397a961aca8ae` | 1,280,835,840 | Installation enabled; current-runtime evidence below |
| Qwen3.5 4B | [`unsloth/Qwen3.5-4B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF) / `Qwen3.5-4B-Q4_K_M.gguf` | `e87f176479d0855a907a41277aca2f8ee7a09523` | 2,740,937,888 | Installation enabled; current-runtime evidence below |

```text
0.8B bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517
2B   aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223
4B   00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4
```

Catalog memory values (2/4/7 GiB) are estimates, not measured peaks. The 2B/4B `verified` catalog flags permit installation; the actual runtime and hardware qualification is recorded below, and the flags do not certify factual correctness. 9B (`qwen35-9b`) is no longer offered; existing inventory remains usable without automatic deletion.

## Current runtime verification

On 2026-11-03, freshly built debug workers (`build_id: development`) passed the eight `validate-worker.mjs` stages on one Windows 11 x64 PC: status, load, ordinary generation, structured generation, terminating/reaping a pending request after 500 ms, reload, recovered generation and shutdown. The 2B/4B model SHA-256 values matched the pinned artifacts above. These are direct worker checks, not installed-app or release-build verification.

No Mac is available for the current implementation work. Metal compilation, actual inference, cancellation, crash recovery and a real CPU fallback remain unverified here; Windows test results cannot establish them.

| Backend | Qwen3.5 2B | Qwen3.5 4B | Observed placement |
| --- | --- | --- | --- |
| Windows CPU | 8 stages passed; 73.784 s total | 8 stages passed; 158.207 s total | `loaded_backend: cpu`, zero GPU layers |
| Windows NVIDIA Vulkan | 8 stages passed; 64.327 s total | 8 stages passed; 92.407 s total | GTX 1650, `loaded_backend: vulkan`; 25 layers for 2B, 31 for 4B |
| macOS Apple Silicon CPU | Not verified | Not verified | No Mac available |
| macOS Apple Silicon Metal | Not verified | Not verified | No Mac available; actual Metal-to-CPU fallback also unverified |

Times are complete validation durations, including file hashing, loading, multiple generations, cancellation, reload and shutdown. They are not single-notice latency or a CPU/GPU performance comparison. Peak RAM was not measured. Cancellation establishes that the parent request was pending when its owned process was terminated, not that a particular native kernel was running. Memory exhaustion and app-managed device fallback were not exercised by these runs.

All four reports record `structure_pass: true`, **`category_pass: false` and `deadline_pass: false`** for the same synthetic scholarship notice. Semantic correctness was explicitly excluded from this integration pass; the results do not establish useful or accurate Korean summaries. The full analysis-controller/Store grounding path was not part of this helper run.

Local reports are `llama-unified-20261103/cpu-2b.json`, `cpu-4b.json`, `vulkan-2b.json` and `vulkan-4b.json`. Both reports for each backend record the same worker SHA-256, also checked against the preserved executable:

```text
CPU     445bac4f5e1d410291a61cdd5aeee51618a53f0b7c2119c7fb936bf6f8f717d5
Vulkan  43813c029b8869ea98af61dc33a7fa404f7c7649f90369f41eb6ffb2f9a61155
```

Earlier macOS CPU/Metal measurements used mistral.rs, and the earlier Windows Vulkan worker used its previous dedicated implementation. Those measurements do not establish speed, memory use or analysis correctness for this unified runtime. The existing [Windows observations](WINDOWS_VALIDATION.md#ai-on-windows) remain limited to the binaries and one GTX 1650 recorded there. Prior Mac measurements and full evaluation history are retained in Git.

0.8B has no real-model evidence. Other NVIDIA GPUs, broad model accuracy and untested operating-system versions remain unverified. CUDA is not a supported build, and Intel Macs are outside product support.

## Known quality limits

Literal grounding catches invented quotes and dates, not wrong meaning. Earlier runtime evaluations found these errors even in schema-valid, grounded output; the runtime change has not established that they are fixed:

- Wrong categories (scholarship notices labeled `학사`; GPU runs returning `공모전·대회`).
- An obsolete or cancelled date kept as a current deadline; the first deadline of a table omitted.
- An excluded group presented as eligible; a date qualifier ("after December 9") dropped.
- A live notice where 2B said submission happened through the homepage and turned "9학기" into "9학점".
- Abbreviated or yearless dates left unnormalized (by design) even when a human could resolve them.

The UI labels every result as AI-generated and tells the user to check conditions and deadlines in the original.

## Constrained decoding decision

Analysis keeps the capability-negotiated `analysis_json_v1` contract: a static closed JSON schema is compiled to llama.cpp grammar, while setup checks use ordinary text. The app still validates schema and exact source evidence and allows one bounded repair. Greedy sampling, disabled thinking and model end-of-generation handling do not guarantee correct categories or dates. Previous mistral.rs A/B results are not a success rate for this grammar implementation; the full 18-case campaign has not been recorded for the unified runtime.

## Running evaluations

Normal tests are network-free and never download weights. These ignored tests need explicit approval for the download and disk budget, and isolated directories, never production app data.

For worker lifecycle checks with an existing model, `scripts/validate-worker.mjs` requires an explicit `--backend cpu|vulkan|metal` and absolute paths:

```sh
node scripts/validate-worker.mjs --backend cpu --worker /absolute/rhythm-ai-worker --model /absolute/model.gguf --report /absolute/new-cpu-report.json
```

It hashes the executable and model, checks status/load, plain and structured generation, terminates only its own child after 500 ms of a pending request, then verifies reloading, generation and shutdown. Reports include actual backends, timings and bounded stderr. CPU must report zero GPU layers; Vulkan needs NVIDIA and positive offload; Metal needs actual Metal offload. An explicit `--allow-cpu-fallback` permits a Metal build's CPU recovery check and labels it `passed_with_cpu_fallback`, never a Metal-only success. `validate-gpu.mjs` remains an alias with `--backend vulkan` fixed.

The integration pass requires valid output structure, not correct summaries, categories or deadlines. Its category/date observations are recorded separately and do not decide the exit status. Mock-only checks of this helper run with `node --test tests/worker-validation.test.mjs`; they are not real inference evidence.

```sh
# Install a pinned artifact through the production downloader into a test directory.
RHYTHM_MODEL_INSTALL_ID=qwen35-2b \
RHYTHM_MODEL_INSTALL_DIR=/tmp/rhythm-model-validation \
cargo test --locked -p rhythm real_curated_model_install -- --ignored --nocapture

# Build and keep the exact worker; Tauri builds can overwrite target/*/rhythm-ai-worker.
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

- `intelligence/evaluation.json` holds 18 synthetic Korean notices (academic, scholarship, multiple dates, tables, yearless and relative dates, long-tail corrections, exclusions and more) with the expected category, deadline set and summary terms. Screening rejects missing and extra deadlines; term matching can flag a valid paraphrase, so human review is still required.
- `RHYTHM_AI_EVAL_LIMIT` defaults to 2 (max 18); `RHYTHM_AI_EVAL_CASES` takes comma-separated IDs. `RHYTHM_AI_EVAL_RECOVERY=1` adds cancellation, worker termination and recovery. `RHYTHM_AI_EVAL_DECODING=unconstrained|analysis_json_v1` overrides the decoding mode for A/B runs. `RHYTHM_AI_EVAL_TRACE=<file>` writes raw generations (test-only; may contain source text).
- A CPU build omits `--features metal` and must be annotated `cpu`; `RHYTHM_AI_EVAL_BACKEND` is a label that must match the real build, not a backend switch.
- `actual_analysis_latency_benchmark` times cold-model, warm and cached analysis on one input; set `RHYTHM_AI_BENCH_LABEL` and `RHYTHM_AI_EVAL_REPORT`.
- `archived_evaluation.rs::actual_archived_notice_evaluation` copies up to ten selected revisions (`RHYTHM_AI_EVAL_ARCHIVE` = a read-only SQLite file, `RHYTHM_AI_ARCHIVE_IDS`) from an existing archive into a fresh store. It never opens the source with `Store::open` and creates no HTTP client.

Reports and traces from archived notices contain source text: keep them local, uncommitted and unuploaded.
