# AI evaluation

What local AI has been verified to do, the pinned artifact and how to run the opt-in evaluations. Installation integrity, runtime embedding, classification accuracy and native behavior are separate claims.

AI summaries (Qwen3.5 2B/4B and Gemma 4 E2B/E4B generation, optional vision projectors) and the OpenAI-compatible remote engine were retired (#226); their evaluation evidence is in git history before that change. Calendar dates come from the deterministic extractor (`archive/dates.rs`), measured in [the roadmap](ROADMAP.md#current-state).

## Runtime

Inference runs in upstream `llama-server` from llama.cpp build **b12068** (`0.8.0-dev`, commit `4402e6540`), pinned with per-archive size and SHA-256 in `scripts/llama-cpp.json`: `macos-arm64` (Metal), `win-vulkan-x64` and `win-cpu-x64`. It runs only as an embedding server (`--embedding`, with the model's pooling mode); no remote tokenizer is used. How Rhythm runs it is in [architecture](ARCHITECTURE.md#inference-process).

## Pinned artifacts

`src-tauri/src/intelligence/catalog.json` is the executable manifest; its checksum comes from the pinned repository LFS metadata. One-click installation accepts only `verified` entries.

| Model | Repository / file | Revision | Bytes | Status |
| --- | --- | --- | ---: | --- |
| Qwen3-Embedding 0.6B | [`Qwen/Qwen3-Embedding-0.6B-GGUF`](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF) / `Qwen3-Embedding-0.6B-Q8_0.gguf` | `370f27d7550e0def9b39c1f16d3fbaa13aa67728` | 639,150,592 | Notice classification; macOS Metal and CPU (18 fixtures) |

```text
Embedding 06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439
```

The catalog memory value (2 GiB) is an estimate, not a measured peak. `verified` means the qualified artifact/backend evidence below, not classification correctness.

## Embedding classification

`intelligence/classification.rs` embeds the instruction, title and first 1000 body characters, then compares the unit vector against 35 authored Korean prototypes (five per non-`기타` category) using the best (max) cosine. The best match below `0.50` becomes `기타` with a low-confidence marker. The prototype set, input shape, scoring and threshold form the config hash stored with each classification; vectors are stored per revision and embedding-model SHA-256 so a prototype change reclassifies without re-embedding. The tracing/evaluation harness is the ignored `actual_embedding_classification_evaluation`, which uses the same 18 synthetic fixtures and never touches the production profile.

The figures below were recorded with a private copy of the prototypes in the harness that differed from the production table in one character (`학생证` for `학생증` in one 학사 prototype). The harness now uses the production table (`classification::CATEGORY_PROTOTYPES`) and the numbers were not re-measured.

Measured on one Mac (macOS 27.2, M1 Pro, 16 GiB) with the staged `llama-server`, three candidate artifacts downloaded into a temporary directory and verified against their pinned size/SHA-256. Input variants grew from title only (0 body characters) to title + 1500 characters; the table reports the best variant per model.

| Model | Pinned artifact | Dim | Category | Time / notice (Metal) | Peak RSS | Load |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| **Qwen3-Embedding 0.6B Q8_0** | `Qwen/Qwen3-Embedding-0.6B-GGUF` `370f27d…`, 639,150,592 B | 1024 | **18/18** (title + 300 chars) | 73 ms | 1.7 GiB | 0.8 s |
| bge-m3 Q8_0 | `gpustack/bge-m3-GGUF` `2d48f17…`, 634,553,760 B | 1024 | 17/18 (`ambiguous` → wrong) | 40–58 ms | 0.84 GiB | 0.9 s |
| multilingual-e5-small | `rodion-m/multilingual-e5-small-gguf` `a4ad957…` fp32, 476,369,248 B | 384 | 6/18 | 5–8 ms | 0.61 GiB | 0.4 s |

The `cstr` and `TwinSunsLLC` Q8_0 e5-small conversions failed to load in b12068 (`bert model needs to define token type count`); only the `rodion-m` fp32 conversion loaded. Qwen3-Embedding was also run on macOS CPU (`-ngl 0 --device none`): 18/18 at title + 300 characters, 237 ms/notice, 2.2 GiB peak RSS. These are synthetic fixtures and authored prototypes, not a broad Korean accuracy rate; Windows CPU/Vulkan classification is unverified. bge-m3 and e5-small are not shipped.

To reproduce, download a candidate into an isolated directory and run:

```sh
RHYTHM_EMBEDDING_EVAL_MODEL=/tmp/qwen3-embedding-0.6b.gguf \
RHYTHM_EMBEDDING_EVAL_ID=qwen3 RHYTHM_EMBEDDING_EVAL_POOLING=last \
RHYTHM_EMBEDDING_EVAL_PREFIX=qwen RHYTHM_EMBEDDING_EVAL_REPORT=/tmp/qwen3.json \
cargo test --locked --release -p rhythm actual_embedding_classification_evaluation -- --ignored --nocapture
```

Set `RHYTHM_EMBEDDING_EVAL_FORCE_CPU=1` for the CPU launch. The report includes every case's expected/predicted category, score, margin and elapsed time.

## Semantic search

Search queries are compared with the stored classifier vectors, so the query instruction and cut-off were chosen on real output (#127). The ignored `actual_semantic_search_evaluation` embeds the 18 fixtures exactly as the classifier stores them (`classification::input`) and 12 authored paraphrase queries (for example `생활비 지원` → the scholarship fixtures, `학교를 잠시 쉬고 싶어요` → `academic`) that mostly avoid the fixtures' own words. Measured on the same Mac with the pinned Qwen3-Embedding 0.6B Q8_0 (verified size/SHA-256, isolated directory) on Metal:

| Query instruction | MRR | Relevant in top 3 |
| --- | ---: | ---: |
| **Qwen3 web search (shipped `SEARCH_INSTRUCTION`)** | **0.882** | 11/12 |
| Korean notice retrieval | 0.826 | 11/12 |
| The classifier instruction | 0.826 | 11/12 |
| None | 0.764 | 9/12 |

Relevant and irrelevant scores overlap (shipped instruction: relevant median 0.31, irrelevant 90th percentile 0.37), so no absolute threshold separates them. Keeping hits within 0.08 of the best score and at least 0.20 kept a relevant notice for all 12 queries, 20 of 23 relevant pairs and 30 irrelevant ones (about 2.5 per query). Through the store's fused ranking (`archive_ranked_query`), a relevant fixture was in the top five for 12/12 queries, against 1/12 for text-only ranking, where 11 queries matched no text. Broad queries still bring in loosely related notices (up to eight hits within the margin). These are synthetic notices and queries; ranking quality on the real archive, on Windows and with other embedding models is unverified.

```sh
RHYTHM_EMBEDDING_EVAL_MODEL=/tmp/qwen3-embedding-0.6b.gguf RHYTHM_EMBEDDING_EVAL_REPORT=/tmp/search.json \
cargo test --locked --release -p rhythm actual_semantic_search_evaluation -- --ignored --nocapture
```

## Running evaluations

Normal tests are network-free and never download weights. These ignored tests need explicit approval for the download and disk budget, and isolated directories, never production app data.

```sh
# Install the pinned artifact through the production downloader into a test directory.
RHYTHM_MODEL_INSTALL_ID=qwen3-embedding-0-6b \
RHYTHM_MODEL_INSTALL_DIR=/tmp/rhythm-model-validation \
cargo test --locked -p rhythm real_curated_model_install -- --ignored --nocapture
```

The classification and search evaluations above need the staged llama.cpp build (`node scripts/stage-llama.mjs`; `RHYTHM_LLAMA_TEST_DIR` can name another, such as a bundle's `Rhythm.app/Contents/Resources/llama`). `intelligence/evaluation.json` holds the 18 synthetic Korean notices (academic, scholarship, multiple dates, tables, yearless and relative dates, long-tail corrections, exclusions and more) with their expected category and deadline set; the date extractor's tests read the same file.
