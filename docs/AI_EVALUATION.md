# AI evaluation

What local AI has been verified to do, the pinned artifacts, and how to run the opt-in evaluations. The analysis contract (prompt, grounding, repair, cache identity) is in [architecture](ARCHITECTURE.md#analysis-validation-and-cache). Installation integrity, runtime generation, source grounding, semantic usefulness and native behavior are separate claims.

## Runtime

Inference runs in upstream `llama-server` from llama.cpp build **b12068** (`0.8.0-dev`, commit `4402e6540`), pinned with per-archive size and SHA-256 in `scripts/llama-cpp.json`: `macos-arm64` (Metal), `win-vulkan-x64` and `win-cpu-x64`. It loads text-only Qwen3.5 GGUF using the file's embedded chat template (`--jinja`), with no remote tokenizer or vision projector. How Rhythm runs it is in [architecture](ARCHITECTURE.md#inference-process).

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

**llama-server b12068, Metal, 2B** (all 18 synthetic fixtures, production controller → validation → Store path, `llama-server` from a bundled debug `Rhythm.app`): 18/18 jobs completed with schema-valid, grounded output, 1 needing its one repair; 7/18 also passed every screening check (category, deadline set, summary terms). 8 had the wrong category and 2 the wrong deadline set. Total inference 92 s (≈65 completion tok/s, 2–10 s per notice); `offloaded 25/25 layers to GPU`. In the same harness and fixtures, the former mistral.rs Metal release worker completed 18/18 and passed 7/18 in 236 s.

Evidence from the former mistral.rs worker (same artifacts, different engine), not re-run with llama-server:

- **4B on Metal** produced useful, validated Korean analysis of synthetic notices.
- **v1.0.0 installed app**, live notice, 2B/Metal: analysis completed in 19.8 s and was cached; Settings showed the loaded backend as Metal.
- **Native flows (2B/Metal, isolated profiles):** download cancel/resume, setup, reader analysis, OS-network-denied regeneration, cancellation and worker-crash recovery, Eco idle unload, load progress surviving a Settings remount.

**llama-server b12068, Windows 11, GTX 1650:** 2B and 4B on Vulkan and 2B on CPU completed 18/18 fixtures, 4B on CPU the first 4; an isolated installed app analyzed one synthetic notice on GPU and CPU. Details in [Windows validation](WINDOWS_VALIDATION.md#ai-on-windows).

Not verified with llama-server: 4B and CPU inference on macOS, the macOS installed app's native AI flows, and cancellation or crash recovery in any installed app. Never verified: 0.8B, NVIDIA GPUs other than one GTX 1650, Intel Macs, and any broad accuracy rate.

## Known quality limits

Literal grounding catches invented quotes and dates, not wrong meaning. Observed on schema-valid, grounded output:

- Wrong categories (scholarship and research notices labeled `학사`; GPU runs returning `공모전·대회`).
- An obsolete or cancelled date kept as a current deadline; the first deadline of a table omitted.
- An excluded group presented as eligible; a date qualifier ("after December 9") dropped.
- A live notice where 2B said submission happened through the homepage and turned "9학기" into "9학점".
- Abbreviated or yearless dates left unnormalized (by design) even when a human could resolve them.

The UI labels every result as AI-generated and tells the user to check conditions and deadlines in the original.

## Constrained decoding decision

Analysis sends a static closed JSON schema as the request's `response_format`; llama.cpp compiles it to a grammar. The grammar emits properties in schema order, so the schema is literal JSON with the summary fields before `facts`. Sent with alphabetically sorted keys (`category` and `facts` first), the same 2B run completed only 14/18: four jobs failed date grounding after their repair because facts were written before the summaries. With the former worker, a same-input A/B test (8 synthetic + 3 archived notices, 2B and 4B) raised completed jobs from 17/22 to 20/22 under constrained decoding. Category and deadline errors remain in every mode.

## Running evaluations

Normal tests are network-free and never download weights. These ignored tests need explicit approval for the download and disk budget, and isolated directories, never production app data.

```sh
# Install a pinned artifact through the production downloader into a test directory.
RHYTHM_MODEL_INSTALL_ID=qwen35-2b \
RHYTHM_MODEL_INSTALL_DIR=/tmp/rhythm-model-validation \
cargo test --locked -p rhythm real_curated_model_install -- --ignored --nocapture

# Uses the staged llama.cpp build; RHYTHM_LLAMA_TEST_DIR can name another,
# such as a bundle's Rhythm.app/Contents/Resources/llama.
node scripts/stage-llama.mjs
RHYTHM_MODEL_EVAL_ROOT=/tmp/rhythm-model-validation \
RHYTHM_MODEL_EVAL_ID=qwen35-2b \
RHYTHM_AI_EVAL_BACKEND=metal \
RHYTHM_AI_EVAL_CASES=scholarship RHYTHM_AI_EVAL_LIMIT=1 \
RHYTHM_AI_EVAL_REPORT=/tmp/rhythm-evaluation-2b-metal.json \
cargo test --locked --release -p rhythm actual_qwen35_analysis_evaluation -- --ignored --nocapture
```

- `intelligence/evaluation.json` holds 18 synthetic Korean notices (academic, scholarship, multiple dates, tables, yearless and relative dates, long-tail corrections, exclusions and more) with the expected category, deadline set and summary terms. Screening rejects missing and extra deadlines; term matching can flag a valid paraphrase, so human review is still required.
- `RHYTHM_AI_EVAL_LIMIT` defaults to 2 (max 18); `RHYTHM_AI_EVAL_CASES` takes comma-separated IDs. `RHYTHM_AI_EVAL_RECOVERY=1` adds cancellation, server termination and recovery. `RHYTHM_AI_EVAL_DECODING=unconstrained|analysis_json_v1` overrides the decoding mode for A/B runs. `RHYTHM_AI_EVAL_TRACE=<file>` writes raw generations (test-only; may contain source text).
- `RHYTHM_AI_EVAL_BACKEND` is a label that must match the device the run actually used (check `loaded_backend` in the report), not a backend switch.
- `actual_analysis_latency_benchmark` times cold-model, warm and cached analysis on one input; set `RHYTHM_AI_BENCH_LABEL` and `RHYTHM_AI_EVAL_REPORT`.
- `archived_evaluation.rs::actual_archived_notice_evaluation` copies up to ten selected revisions (`RHYTHM_AI_EVAL_ARCHIVE` = a read-only SQLite file, `RHYTHM_AI_ARCHIVE_IDS`) from an existing archive into a fresh store. It never opens the source with `Store::open` and creates no HTTP client.

Reports and traces from archived notices contain source text: keep them local, uncommitted and unuploaded.
