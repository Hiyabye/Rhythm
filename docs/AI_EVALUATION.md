# AI evaluation

What Rhythm's AI has been verified to do: the collector's server embeddings, the pinned local artifact and how to run the opt-in evaluations. Installation integrity, runtime embedding, classification accuracy and native behavior are separate claims.

Since v2.0.0 the collector embeds and classifies public notices through Cloudflare Workers AI and ships vectors and categories in the feed; the app embeds only search queries and Cybercampus notices with the local model (#166 D5, D6, D12, D25). Both sides use the same model, input builder, prototypes and thresholds (`crates/embed`), so a local query vector is compared directly with feed vectors.

AI summaries (Qwen3.5 2B/4B and Gemma 4 E2B/E4B generation, optional vision projectors) and the OpenAI-compatible remote engine were retired (#226); their evaluation evidence is in git history before that change. Calendar dates come from the deterministic extractor (`crates/sources/src/dates.rs`), measured in [the roadmap](ROADMAP.md#current-state).

## Runtime

Inference runs in upstream `llama-server` from llama.cpp build **b12068** (`0.8.0-dev`, commit `4402e6540`), pinned with per-archive size and SHA-256 in `scripts/llama-cpp.json`: `macos-arm64` (Metal) and `win-cpu-x64`. It runs only as an embedding server (`--embedding`, with the model's pooling mode); no remote tokenizer is used. How Rhythm runs it is in [architecture](ARCHITECTURE.md#inference-process).

## Server embedding (Workers AI)

The collector sends the exact `rhythm_embed::input` strings (instruction prefix + title + first 1,000 body characters) through the `text` field of `@cf/qwen/qwen3-embedding-0.6b`, never the `queries`/`instruction` fields, which format the prompt differently (cosine 0.973 to the app's query vector, #166 D26). Cloudflare's [data usage page](https://developers.cloudflare.com/workers-ai/platform/data-usage/) states that Workers AI inputs and outputs are not used for training and are not retained after the request (checked 2026-11-20); the collector sends public notice text only and uses no AI Gateway.

**Equivalence with the local model** (#170, 2026-11-17, one Mac; reference = local Metal f32, baseline = everything on local CPU): 413 notices (325 from a copied archive, 26 detail fixtures of the other board families, the 18 synthetic cases), the 78 prototypes and 42 queries.

| | Local CPU (baseline) | Workers AI notices/prototypes + Metal queries |
| --- | --- | --- |
| Same-text cosine min / median (491 texts) | 0.99740 / 0.99962 | 0.99876 / 0.99968 |
| Category changes at 0.68 (of 413) | 6 | 2 |
| Search top-10 overlap mean / min (42 queries) | 9.55 / 8 | 9.76 / 9 |
| Pairs crossing 0.75 / 0.97 (of 85,078) | 413 / 29 | 264 / 9 |

Token counts matched for every checked text. Workers AI differs from local Metal less than local CPU does, so the v1 thresholds (0.68 category, 0.20/0.08 search, 0.75 similar, 0.97 duplicate) carry over.

**Vector encoding:** the feed carries int8 × 1,024 with a per-vector f32 scale (1,028 bytes, [FEED.md](FEED.md)). Encoded the same way on both sides, it stayed within the CPU baseline (2 category changes, top-10 9.69 / 8, 268 / 8 crossings); int8 alone keeps cosine ≥ 0.99986 to f32. Vectors truncated to 512 or 256 dimensions failed with the v1 thresholds (12–24 category changes, top-10 below 8) and are not used.

**Drift canary:** Cloudflare can change the hosted model without notice. Once per UTC day the collector re-embeds 28 committed canary inputs (`crates/collector/src/canary.json`: the 18 synthetic cases with the classifier prefix and 10 queries with the search prefix) and compares them with the first recorded reference: min cosine ≥ 0.9995 is normal, 0.9970–0.9995 warns, < 0.9970 stops new vectors until the maintainer resets it. Three same-day runs over 11.7 hours on 2026-11-17 gave cosine ≈ 1.0, batched or not.

**Not measured:** Windows local query vectors against Workers AI (the Windows comparison was dropped by maintainer decision, 2026-11-17; a small difference there affects query ranking only); category accuracy of feed notices beyond the copied-archive evaluation below.

## Pinned artifacts

`src-tauri/src/intelligence/catalog.json` is the executable manifest; its checksum comes from the pinned repository LFS metadata. One-click installation accepts only `verified` entries. The Qwen3-Embedding weights are Apache-2.0 and are never bundled with the app.

| Model | Repository / file | Revision | Bytes | Status |
| --- | --- | --- | ---: | --- |
| Qwen3-Embedding 0.6B | [`Qwen/Qwen3-Embedding-0.6B-GGUF`](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF) / `Qwen3-Embedding-0.6B-Q8_0.gguf` | `370f27d7550e0def9b39c1f16d3fbaa13aa67728` | 639,150,592 | Search queries and Cybercampus classification; macOS Metal and CPU (18 fixtures) |

```text
Embedding 06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439
```

The catalog memory value (2 GiB) is an estimate, not a measured peak. `verified` means the qualified artifact/backend evidence below, not classification correctness.

## Embedding classification

`intelligence/classification.rs` embeds the instruction, title and first 1000 body characters, then compares the unit vector against 78 authored Korean prototypes (the instruction, input builder, prototypes and scoring live in `crates/embed`) for the ten non-`기타` categories using the best (max) cosine. Prototypes are embedded with the same instruction prefix as notices. The best match below `0.68` becomes `기타` with a low-confidence marker. The prototype set, input shape, scoring and threshold form the config hash stored with each classification; vectors are stored per revision and embedding-model SHA-256 so a prototype change reclassifies without re-embedding. The tracing/evaluation harness is the ignored `actual_embedding_classification_evaluation` (run it with `RHYTHM_EMBEDDING_EVAL_PREFIX=qwen` to match production), which uses the same 18 synthetic fixtures and never touches the production profile.

### Archived notices

Measured on 2026-11-16 on a read-only copy of one real profile: 1,073 latest-revision notices from the computing, main, dormitory and eClass boards, labeled by hand (an alternate label where two categories fit). The labels are personal data and are not committed. Scoring reused the stored vectors with the pinned Qwen3-Embedding 0.6B Q8_0 on Metal; re-embedding the notices with an instruction naming the new categories scored the same, so the instruction was kept.

| Configuration | Correct |
| --- | ---: |
| Previous: 8 categories, 35 prototypes embedded without the prefix, cut-off 0.50 (new categories folded into the old ones) | 69.3% |
| 8 categories, new prototypes embedded with the prefix | 86.7% |
| **Shipped: 11 categories, 78 prototypes with the prefix, cut-off 0.68** | **91.6%** (983/1073) |

Without the prefix, scores sat between 0.5 and 0.7 with margins near zero; every dormitory menu became 공모전·대회 or 연구 and dormitory admission notices became 연구. The previous prototypes were close paraphrases of the synthetic fixture titles, which is why the fixture table below reached 18/18. The prototypes were written while reading these same notices, so 91.6% is optimistic for unseen notices. Remaining errors are mostly boundary cases: job-linked training programs (교육·프로그램 vs 인턴·채용), external conferences (행사·활동 vs 공모전·대회) and 벨라르미노 학사 (the dormitory) notices drawn to 학사. Below 0.68, about half of the top matches were wrong. On the 18 fixtures the shipped configuration scores 15/18 (title + 1000 characters, max): `formatted_table` (공모전·대회 → 연구), `long` (교육·프로그램 → 수업) and `long_tail_correction` (교육·프로그램 → 학사).

### Model comparison

The figures below were recorded with the previous prototype table before the prefix change and were not re-measured.

Measured on one Mac (macOS 27.2, M1 Pro, 16 GiB) with the staged `llama-server`, three candidate artifacts downloaded into a temporary directory and verified against their pinned size/SHA-256. Input variants grew from title only (0 body characters) to title + 1500 characters; the table reports the best variant per model.

| Model | Pinned artifact | Dim | Category | Time / notice (Metal) | Peak RSS | Load |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| **Qwen3-Embedding 0.6B Q8_0** | `Qwen/Qwen3-Embedding-0.6B-GGUF` `370f27d…`, 639,150,592 B | 1024 | **18/18** (title + 300 chars) | 73 ms | 1.7 GiB | 0.8 s |
| bge-m3 Q8_0 | `gpustack/bge-m3-GGUF` `2d48f17…`, 634,553,760 B | 1024 | 17/18 (`ambiguous` → wrong) | 40–58 ms | 0.84 GiB | 0.9 s |
| multilingual-e5-small | `rodion-m/multilingual-e5-small-gguf` `a4ad957…` fp32, 476,369,248 B | 384 | 6/18 | 5–8 ms | 0.61 GiB | 0.4 s |

The `cstr` and `TwinSunsLLC` Q8_0 e5-small conversions failed to load in b12068 (`bert model needs to define token type count`); only the `rodion-m` fp32 conversion loaded. Qwen3-Embedding was also run on macOS CPU (`-ngl 0 --device none`): 18/18 at title + 300 characters, 237 ms/notice, 2.2 GiB peak RSS. These are synthetic fixtures and authored prototypes, not a broad Korean accuracy rate. Windows CPU classification ran once on one PC (36 Cybercampus notices, #271, [Windows validation](WINDOWS_VALIDATION.md#v2-feed-cpu-classifier-and-release-build-271)); its accuracy was not measured. bge-m3 and e5-small are not shipped.

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

Relevant and irrelevant scores overlap (shipped instruction: relevant median 0.31, irrelevant 90th percentile 0.37), so no absolute threshold separates them. Keeping hits within 0.08 of the best score and at least 0.20 kept a relevant notice for all 12 queries, 20 of 23 relevant pairs and 30 irrelevant ones (about 2.5 per query). Through the store's fused ranking (`archive_ranked_query`), a relevant fixture was in the top five for 12/12 queries, against 1/12 for text-only ranking, where 11 queries matched no text. Broad queries still bring in loosely related notices (up to eight hits within the margin). These are synthetic notices and queries; ranking quality on the real archive, on Windows (where one paraphrase query returned meaning-only results, #271) and with other embedding models is unverified.

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
