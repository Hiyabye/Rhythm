# AI evaluation

What local AI has been verified to do, the pinned artifacts, and how to run the opt-in evaluations. The analysis contract (prompt, grounding, repair, cache identity) is in [architecture](ARCHITECTURE.md#analysis-validation-and-cache). Installation integrity, runtime generation, source grounding, semantic usefulness and native behavior are separate claims.

## Runtime

The CPU/Metal worker pins mistral.rs **v0.9.1**, release commit [`11f255058573e085afe0a149f8a19cbd39aa1200`](https://github.com/EricLBuehler/mistral.rs/tree/11f255058573e085afe0a149f8a19cbd39aa1200). Earlier 0.8.1/0.9.0 GGUF routes lacked `qwen35`; this release loads text-only Qwen3.5 GGUF without a vision projector, using the file's embedded tokenizer and chat template (no remote tokenizer or token). The Windows GPU worker is llama.cpp over Vulkan; see [Windows](WINDOWS.md#gpu-worker).

## Pinned artifacts

`src-tauri/src/intelligence/catalog.json` is the executable manifest: Unsloth GGUF conversions of the official Qwen3.5 models, Apache-2.0, checksums from the pinned repository LFS metadata. One-click installation accepts only `verified` entries.

| Model | Repository / file | Revision | Bytes | Status |
| --- | --- | --- | ---: | --- |
| Qwen3.5 0.8B | [`unsloth/Qwen3.5-0.8B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF) / `Qwen3.5-0.8B-Q4_K_M.gguf` | `6ab461498e2023f6e3c1baea90a8f0fe38ab64d0` | 532,517,120 | Experimental: never downloaded or run |
| Qwen3.5 2B | [`unsloth/Qwen3.5-2B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-2B-GGUF) / `Qwen3.5-2B-Q4_K_M.gguf` | `f6d5376be1edb4d416d56da11e5397a961aca8ae` | 1,280,835,840 | Verified for macOS Apple Silicon / Metal |
| Qwen3.5 4B | [`unsloth/Qwen3.5-4B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF) / `Qwen3.5-4B-Q4_K_M.gguf` | `e87f176479d0855a907a41277aca2f8ee7a09523` | 2,740,937,888 | Verified for macOS Apple Silicon / Metal |

```text
0.8B bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517
2B   aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223
4B   00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4
```

Catalog memory values (2/4/7 GiB) are estimates, not measured peaks. `verified` means the qualified artifact/backend evidence below, not factual correctness. 9B (`qwen35-9b`) was removed from the catalog after v1.0.0 without ever being run; existing installs keep working.

## Current evidence (macOS 27.2, M1 Pro, 16 GiB)

- **2B and 4B on Metal** produced useful, validated Korean analysis of synthetic notices through the real worker → controller → validation → Store path, including cache reuse, revision isolation and cached reads with the worker stopped.
- **Release worker, 2B** (copied from the v0.9.2 release bundle, scholarship case, 1,024 output tokens): Metal ≈44 completion tok/s, ≈12.8 s warm and ≈14.6 s cold-model analysis, worker RSS ≈3.0 GB. A CPU release worker: ≈28–30 tok/s, ≈21–24 s, RSS 3.6–4.6 GB. Both outputs were schema-valid and grounded with the right dates, but both labeled the category `학사` instead of `장학`.
- **v1.0.0 installed app**, live notice, 2B/Metal: analysis completed in 19.8 s and was cached; Settings showed the loaded backend as Metal.
- **Native flows (2B/Metal, isolated profiles):** download cancel/resume, setup, reader analysis, OS-network-denied regeneration, cancellation and worker-crash recovery, Eco idle unload, load progress surviving a Settings remount.
- **Windows:** CPU and GTX 1650 GPU observations are in [Windows validation](WINDOWS_VALIDATION.md#ai-on-windows).

Not verified: 0.8B, 4B on CPU (an early CPU load failed under memory pressure), CUDA, NVIDIA GPUs other than one GTX 1650, Intel Macs, and any broad accuracy rate.

## Known quality limits

Literal grounding catches invented quotes and dates, not wrong meaning. Observed on schema-valid, grounded output:

- Wrong categories (scholarship notices labeled `학사`; GPU runs returning `공모전·대회`).
- An obsolete or cancelled date kept as a current deadline; the first deadline of a table omitted.
- An excluded group presented as eligible; a date qualifier ("after December 9") dropped.
- A live notice where 2B said submission happened through the homepage and turned "9학기" into "9학점".
- Abbreviated or yearless dates left unnormalized (by design) even when a human could resolve them.

The UI labels every result as AI-generated and tells the user to check conditions and deadlines in the original.

## Constrained decoding decision

Analysis uses the capability-negotiated `analysis_json_v1` mode (a static closed JSON schema in the worker). In a same-input A/B test (8 synthetic + 3 archived notices per model, 2B and 4B, Metal release worker), completed jobs rose from 17/22 to 20/22 with no material timing difference. Category and deadline errors remained in both modes. The worker stops at `<|endoftext|>` in this mode; without it, a 2B trial repeated grammar-EOS tokens until the output limit. The full 18-case campaign has not been rerun under the final default, so no final acceptance rate exists.

## Running evaluations

Normal tests are network-free and never download weights. These ignored tests need explicit approval for the download and disk budget, and isolated directories, never production app data.

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
