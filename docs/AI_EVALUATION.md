# AI evaluation

What local AI has been verified to do, the pinned artifacts, and how to run the opt-in evaluations. The analysis contract (prompt, grounding, repair, cache identity) is in [architecture](ARCHITECTURE.md#analysis-validation-and-cache). Installation integrity, runtime generation, source grounding, semantic usefulness and native behavior are separate claims.

## Runtime

Inference runs in upstream `llama-server` from llama.cpp build **b12068** (`0.8.0-dev`, commit `4402e6540`), pinned with per-archive size and SHA-256 in `scripts/llama-cpp.json`: `macos-arm64` (Metal), `win-vulkan-x64` and `win-cpu-x64`. It loads text-only Qwen3.5 and Gemma 4 GGUF using the file's embedded chat template (`--jinja`), with no remote tokenizer or vision projector. How Rhythm runs it is in [architecture](ARCHITECTURE.md#inference-process).

## Pinned artifacts

`src-tauri/src/intelligence/catalog.json` is the executable manifest: Unsloth GGUF conversions of the official Qwen3.5 and Gemma 4 models, Apache-2.0, checksums from the pinned repository LFS metadata. One-click installation accepts only `verified` entries.

| Model | Repository / file | Revision | Bytes | Status |
| --- | --- | --- | ---: | --- |
| Qwen3.5 2B | [`unsloth/Qwen3.5-2B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-2B-GGUF) / `Qwen3.5-2B-Q4_K_M.gguf` | `f6d5376be1edb4d416d56da11e5397a961aca8ae` | 1,280,835,840 | Verified for macOS Apple Silicon / Metal and macOS CPU; Windows GTX 1650 / Vulkan and Windows CPU |
| Qwen3.5 4B | [`unsloth/Qwen3.5-4B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF) / `Qwen3.5-4B-Q4_K_M.gguf` | `e87f176479d0855a907a41277aca2f8ee7a09523` | 2,740,937,888 | Verified for macOS Apple Silicon / Metal (18 fixtures); Windows GTX 1650 / Vulkan and Windows CPU (4 of 18 fixtures) |
| Gemma 4 E2B | [`unsloth/gemma-4-E2B-it-GGUF`](https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF) / `gemma-4-E2B-it-Q4_K_M.gguf` | `0314792d7f1f7e229411f620751375812bb9faf2` | 3,106,738,272 | Verified for macOS Apple Silicon / Metal (18 fixtures) |
| Gemma 4 E4B | [`unsloth/gemma-4-E4B-it-GGUF`](https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF) / `gemma-4-E4B-it-Q4_K_M.gguf` | `bfc15c382204943c3a8fff0c750b94ae2364d7a3` | 4,977,171,584 | Verified for macOS Apple Silicon / Metal (18 fixtures) |

```text
2B    aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223
4B    00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4
E2B   740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8
E4B   85a896a047553e842f25297ee5b031d64ff30147d9c4af17b1e4b394cd1fab87
```

Catalog memory values (4/7 GiB for Qwen3.5, 6/9 GiB for Gemma 4) are estimates, not measured peaks. `verified` means the qualified artifact/backend evidence below, not factual correctness. 9B (`qwen35-9b`) was removed from the catalog after v1.0.0 without ever being run, and 0.8B (`qwen35-0-8b`, [`unsloth/Qwen3.5-0.8B-GGUF`](https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF) `Qwen3.5-0.8B-Q4_K_M.gguf` at `6ab461498e2023f6e3c1baea90a8f0fe38ab64d0`, SHA-256 `bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517`) after failing the evaluation below; existing installs keep working.

## Current evidence

**llama-server b12068, Metal, 2B** (macOS 27.2, M1 Pro, 16 GiB; all 18 synthetic fixtures, production controller → validation → Store path, `llama-server` from a bundled debug `Rhythm.app`): 18/18 jobs completed with schema-valid, grounded output, 1 needing its one repair; 7/18 also passed every screening check (category, deadline set, summary terms). 8 had the wrong category and 2 the wrong deadline set. Total inference 92 s (≈65 completion tok/s, 2–10 s per notice); `offloaded 25/25 layers to GPU`. In the same harness and fixtures, the former mistral.rs Metal release worker completed 18/18 and passed 7/18 in 236 s.

**llama-server b12068, CPU, 2B (#82)** (same Mac, harness and 18 fixtures; the same pinned file, size and SHA-256 matched; the evaluation worker selected the CPU path directly with `RHYTHM_AI_EVAL_FORCE_CPU=1`, the same `-ngl 0 --device none` launch the app uses for a fallback session): the server logged `offloaded 0/25 layers to GPU` and 6 CPU threads, the worker reported `실제 불러온 백엔드: CPU`, and every result recorded `loaded_backend: cpu`. 18/18 jobs completed with schema-valid, grounded output, 1 needing its one repair; 7/18 passed every screening check. 7 had the wrong category and 4 the wrong deadline set (Metal produced 8 and 2), `internship` missed the term "지원" and `long_tail_correction` "정정". The 18 requests took 172.6 s of generation wall time and 130.9 s of token time for 5,422 completion tokens (≈41 completion tok/s; Metal reported ≈65) and 185.3 s total wall (3.8–24.9 s per notice); model load 2.8 s, server RSS ≈3.0 GiB. The persisted-cache checks passed with the worker stopped.

**llama-server b12068, Metal, 4B** (same Mac, harness and 18 fixtures; the pinned file downloaded through `real_curated_model_install`, 2,740,937,888 bytes, SHA-256 matched; the staged `llama-server` from `src-tauri/binaries/llama`): 18/18 jobs completed with schema-valid, grounded output and no repair; 15/18 also passed every screening check. Two completed jobs had the wrong category (`no_deadline` and `relative_date` → 학사) and `long_tail_correction` said the deadline "변경" instead of the expected "정정" term, a paraphrase the literal term check flags. Every case's deadline set was exact, including the cancelled first date, tables and relative dates; `incomplete_source` stated that the details are in the unextracted attachment instead of inventing them. Total inference 180 s (153 s generation, 5–15 s per notice, ≈25 completion tok/s); load 2.0 s, server RSS 3.8 GiB after load and 4.1 GiB peak. `loaded_backend` was `metal` for all 18 jobs.

**llama-server b12068, Metal, Gemma 4 E2B** (same Mac, harness and 18 fixtures; the pinned file downloaded through `real_curated_model_install`, 3,106,738,272 bytes, SHA-256 matched; the staged `llama-server`): 18/18 jobs completed with schema-valid, grounded output and no repair; 12/18 passed every screening check. Three categories were wrong (`event_only` and `yearless_deadline` → 수업·교육, `formatted_table` → 연구), two deadline sets were off (`academic` wrote the application period "2026년 2월 2일부터 2026년 2월 20일까지" as one fact with no end date, so 2026-02-20 was missing; `multiple_dates` labeled the certificate-issuance start 2026-09-21 as a deadline), and `long` omitted the term "신청". `incomplete_source` stated that the details are only in the unextracted attachment, with no invented deadline or amount, and `long_tail_correction` kept only the corrected 2026-08-17 deadline. Total inference 131.7 s (≈42 completion tok/s, 5–10 s per notice), load 2.6 s, server RSS ≈3.3 GiB.

**llama-server b12068, Metal, Gemma 4 E4B** (same Mac, harness and 18 fixtures; the pinned file downloaded through `real_curated_model_install`, 4,977,171,584 bytes, SHA-256 matched; the staged `llama-server`): 18/18 jobs completed with schema-valid, grounded output and no repair; 14/18 passed every screening check. Every category was correct. `academic` again reported the period as one range without the end date (2026-02-20 missing); `long_tail_correction` kept the cancelled 2026-08-10 as a second deadline fact even though the summary said it was replaced; `internship` omitted "지원" and `formatted_table` "온라인" from the summary. `incomplete_source` again stayed inside the body. Total inference 238.8 s (≈25 completion tok/s, 7–18 s per notice), load 2.4 s, server RSS ≈5.5 GiB.

**llama-server b12068, Metal, 0.8B, rejected (#74)** (same Mac, harness and 18 fixtures; the pinned file downloaded through `real_curated_model_install`, 532,517,120 bytes, SHA-256 matched): only 14/18 jobs completed; 4 (`no_deadline`, `complex_eligibility`, `relative_date`, `long_tail_correction`) failed validation after their one repair, and 3 completed jobs also needed it. 3/18 passed every screening check; of the 14 completed, 7 had the wrong category and 4 the wrong deadline set. Total inference 61 s (≈111 completion tok/s), server RSS ≈1.1 GiB. Completed results were not useful: for `incomplete_source` (body says only that details are in an unextracted attachment) it saved a summary about a "2024년 1월 1일부터 2월 20일까지의 군사 훈련" and a "2 월 20 일" deadline fact, none of which is in the source; `long` reversed the 휴학생 exclusion and `event_only` turned the event time into a deadline. Windows was not run.

**llama-server b12068, installed macOS app (#83):** a release `Rhythm.app` (`817f25d`, production identifier) launched by its executable with a temporary `HOME` on the populated synthetic fixture profile; the pinned 2B file (size and SHA-256 matched, cloned from an earlier validation profile, so the in-app download was not exercised) was registered as an external file. The app used the loopback fallback (the scratch path exceeds the socket limit) on 127.0.0.1 only, and `run/` was empty afterwards.

- **Load and unload:** selected and loaded on Metal; 메모리 해제 reaped the server.
- **Reader analysis:** with no server loaded, analysis started one and saved grounded results for two fixture notices (2.3 s and 3.8 s, Metal). A third (body "추가 모집 안내") failed validation after its one repair; the reader showed the validator's repair instruction as the error text.
- **Outdated result:** changing 최대 출력 토큰 marked the saved result as differing from the current settings while keeping it readable; regeneration completed (1.9 s).
- **Cancellation:** cancelling while a fresh server was starting reaped it and showed "분석을 취소했습니다"; the job was stored as cancelled and the next request started a new server and completed.
- **Server killed mid-request:** `kill -9` during a request switched the session to CPU (`-ngl 0 --device none`) and the same job completed on CPU (9.1 s total, result `loaded_backend` cpu); Settings showed the CPU switch with a reason that describes an incomplete response rather than a crash. This is one short notice, not a CPU quality check.
- **Quit with a model loaded:** the app's 종료 menu item exited the app and its `llama-server`; no process remained. `integrity_check` ok.

Evidence from the former mistral.rs worker (same artifacts, different engine), not re-run with llama-server:

- **4B on Metal** produced useful, validated Korean analysis of synthetic notices.
- **v1.0.0 installed app**, live notice, 2B/Metal: analysis completed in 19.8 s and was cached; Settings showed the loaded backend as Metal.
- **Native flows (2B/Metal, isolated profiles):** download cancel/resume, setup, reader analysis, OS-network-denied regeneration, cancellation and worker-crash recovery, Eco idle unload, load progress surviving a Settings remount.

**llama-server b12068, Windows 11, GTX 1650:** 2B and 4B on Vulkan and 2B on CPU completed 18/18 fixtures, 4B on CPU the first 4; an isolated installed app analyzed one synthetic notice on GPU and CPU. Details in [Windows validation](WINDOWS_VALIDATION.md#ai-on-windows).

Not verified with llama-server: in the installed macOS app, the in-app download, cancellation during generation (only during server start), a Unix-socket launch and the production profile; cancellation or crash recovery in the installed Windows app. Never verified: NVIDIA GPUs other than one GTX 1650, Intel Macs, and any broad accuracy rate.

## Known quality limits

Literal grounding catches invented quotes and dates, not wrong meaning. Observed on schema-valid, grounded output:

- Wrong categories (scholarship and research notices labeled `학사`; GPU runs returning `공모전·대회`).
- An obsolete or cancelled date kept as a current deadline; the first deadline of a table omitted.
- An excluded group presented as eligible; a date qualifier ("after December 9") dropped.
- A live notice where 2B said submission happened through the homepage and turned "9학기" into "9학점".
- Abbreviated or yearless dates left unnormalized (by design) even when a human could resolve them.
- A date range written as one deadline fact (`2026년 2월 2일부터 2월 20일까지`) can leave the end date unbound, so the deadline set is incomplete; both Gemma 4 runs did this on `academic`.
- An issuance start can be labeled a deadline (`multiple_dates`, E2B), and a replaced date can remain as a second deadline fact (`long_tail_correction`, E4B) even when the summary states the correction.

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
- `RHYTHM_AI_EVAL_FORCE_CPU=1` is a test-only switch: the evaluation tests start the CPU launch (`-ngl 0 --device none`) instead of the Metal GPU launch. It exercises the macOS fallback session without changing application behavior.
- `actual_analysis_latency_benchmark` times cold-model, warm and cached analysis on one input; set `RHYTHM_AI_BENCH_LABEL` and `RHYTHM_AI_EVAL_REPORT`.
- `archived_evaluation.rs::actual_archived_notice_evaluation` copies up to ten selected revisions (`RHYTHM_AI_EVAL_ARCHIVE` = a read-only SQLite file, `RHYTHM_AI_ARCHIVE_IDS`) from an existing archive into a fresh store. It never opens the source with `Store::open` and creates no HTTP client.

Reports and traces from archived notices contain source text: keep them local, uncommitted and unuploaded.
