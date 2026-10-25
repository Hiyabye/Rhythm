# Rhythm AI worker

Rhythm bundles a separate Rust executable, `rhythm-ai-worker`, for local inference.
It starts without weights, retains at most one model, and uses no inference server or
network port. Diagnostic output (including upstream stdout prints) is redirected to
stderr; stdout carries only versioned JSON Lines IPC.

## Runtime and compatibility

The worker pins **mistral.rs v0.9.1**, upstream release commit
[`11f255058573e085afe0a149f8a19cbd39aa1200`](https://github.com/EricLBuehler/mistral.rs/tree/11f255058573e085afe0a149f8a19cbd39aa1200).
The published crates.io version remained 0.8.1 when inspected, so this is a commit-pinned
Git dependency, not an assumed registry 0.9.1 package. Its Candle dependency is also
upstream-pinned and captured by the root lockfile. Only the mistral.rs dependency family
and its necessary transitive resolution were changed.

Why upgrade: inspected 0.8.1 and v0.9.0 GGUF loaders do not route `qwen35`.
The v0.9.1 native GGUF loader explicitly supports `qwen35`; its
`scopes_main_only_multimodal_architectures` test confirms a text-only Qwen3.5 GGUF does
not require a multimodal projector. This is source-level compatibility evidence, not
proof that every quantization/artifact/backend runs correctly. A projector is not
loaded or downloaded by Rhythm's text-only path.

An existing canonical local directory is passed to the GGUF loader. No remote tokenizer
ID is set and `TokenSource::None` is used. Models must contain usable tokenizer/chat
metadata; unsupported or incomplete artifacts return an error rather than fetching
replacement metadata. Neither the builder nor requests enable tools, search, MCP,
shell execution, code execution, or file attachments.

## Build and tests

From the repository root, with Cargo/Rust on PATH:

```sh
cargo build --locked -p rhythm-ai-worker
cargo test --locked -p rhythm-ai-worker
cargo build --locked -p rhythm-ai-worker --release
```

CPU is the default. Optional build features are `metal` and `cuda`; they require
compatible platform developer tooling. A lifecycle-only test build is available with
`--no-default-features`, and explicitly reports `runtime_available: false`. It cannot
perform inference and is not a production substitute. `build_backend` describes the
compiled feature, **not measured GPU utilization or backend verification**. Status and
inference responses separately expose `loaded_backend` (`cpu`, `metal`, `cuda`, or
`unknown`), read from the loaded runtime device; it is null while unloaded. If an
accelerated build fails to initialize/load, it drops that failed attempt and retries
once with a fresh CPU-forced builder. Both attempts are bounded by the parent request
deadline; failure of the CPU attempt is returned, not retried indefinitely. Diagnostics
include the accelerated failure. A successful CPU fallback is never labeled as Metal
or CUDA. Runtime device selection is not itself a GPU-utilization measurement.

```sh
cargo build --locked -p rhythm-ai-worker --release --features metal
cargo build --locked -p rhythm-ai-worker --release --features cuda
```

The root desktop helper also accepts an explicit backend selection:

```sh
RHYTHM_AI_BACKEND=metal npm run desktop
RHYTHM_AI_BACKEND=metal npm run desktop:build
```

Desktop scripts prefer Metal on Apple Silicon if the existing `xcrun metal --version`
probe succeeds; otherwise they select CPU. `RHYTHM_AI_BACKEND` explicitly overrides
this choice with `cpu`, `metal` or `cuda`. It selects the
worker's build feature, not a guarantee that the accelerated device will load a model.
Compatible tooling must already exist; the helper installs no toolchain or weights.
The current useful Qwen3.5 output evidence is for Metal, not CPU.

Do not enable upstream tool/search/execution capabilities or implicit remote resolution
for notice analysis. GPU toolchains and model weights are not installed by these scripts.

## Protocol v1

One UTF-8 JSON request/response per line; framing is bounded to 1 MiB. Requests remain
serial and preserve their numeric ID and version. Unknown versions/methods fail without
terminating the process. Malformed JSON returns ID 0. Oversized frames terminate the
worker. Closing stdin or sending `shutdown` ends it.

```json
{"version":1,"id":1,"method":"ping"}
{"version":1,"id":2,"method":"status"}
{"version":1,"id":3,"method":"load","params":{"model_path":"/absolute/path/model.gguf"}}
{"version":1,"id":4,"method":"infer","params":{"model_path":"/absolute/path/model.gguf","prompt":"분석할 문서와 출력 형식","max_tokens":1024}}
{"version":1,"id":5,"method":"unload"}
{"version":1,"id":6,"method":"shutdown"}
```

Existing `ping`, `status`, `infer`, and `shutdown` envelopes remain compatible; `load`
and `unload` are additive. `infer` still loads/switches on demand when necessary.
Model paths are canonical local `.gguf` files, with GGUF v2/v3 magic/version validation
before passing them to mistral.rs. Full architecture/tensor validation occurs in the
runtime. The previous model is dropped before another is loaded. Reuse requires the
same canonical path, file size and modification timestamp. Changes force a reload;
this metadata check is not an integrity guarantee. The application independently
checks the artifact SHA-256 and unloads on external-file re-registration.

```json
{"version":1,"id":2,"ok":true,"result":{"engine":"mistral.rs","runtime_available":true,"model":null,"load_ms":0,"build_backend":"cpu","loaded_backend":null}}
{"version":1,"id":4,"ok":true,"result":{"text":"실제 생성된 응답","model":"/absolute/path/model.gguf","load_ms":123,"loaded_backend":"cpu","generation_ms":456,"usage":{"completion_tokens":10}}}
{"version":1,"id":4,"ok":false,"error":"한국어 오류 메시지"}
```

The `usage` example is abbreviated: the runtime returns prompt/completion counts and
its timing/throughput fields. These are measurements from that request, not advertised
performance guarantees. `load`/`unload` return status; `ping` returns `healthy`, and
`shutdown` returns `shutdown`.

Inputs are nonempty and at most **24 KiB UTF-8**, outputs 1–4096 tokens (default 128),
with one sequence and a 32,768-token maximum context. Prefix caching is disabled to
bound retained state. Upstream silent truncation is disabled. Oversized inputs fail;
the caller must mark any bounded partial-document processing honestly. No binary
attachment content is accepted. Generation uses the deterministic sampler, disables
thinking, and sends a separate fixed system instruction treating document text as
untrusted data. Output still requires application schema and evidence validation;
a system prompt alone is not a factual-grounding guarantee.

## Application lifecycle

`src-tauri/src/intelligence/mod.rs::Worker` owns Tokio pipes, child process and executable
path. `infer` has a ten-minute deadline, `load` three minutes, and ordinary lifecycle
requests eight seconds. Protocol versions/IDs/shape are validated. A failed inference
or load kills and reaps the worker, leaving the archive/application independent.
A later inference/load restarts it automatically. Explicit restart also performs a
validated health check.

Inference cancellation is **process cancellation**, not an in-band request queued behind
inference: drop the pending future and call `Worker::stop().await` before any further
pipe use. This cancels the current operation and releases its model memory. A subsequent
request reloads once; normal completed requests reuse the worker/model. Application
`unload` restarts the empty worker to guarantee the OS reclaims upstream allocations.
The direct protocol `unload` drops the model; OS-level memory release is guaranteed only
by reaping its process. Shutdown requests clean exit before forced termination; child
processes are kill-on-drop as a final safeguard. stderr drains bounded chunks even if
upstream emits invalid UTF-8 or no newline.

## Actual-model verification

Routine tests do not download weights. Opt in with an existing artifact:

```sh
RHYTHM_TEST_MODEL=/absolute/path/model.gguf cargo test --locked -p rhythm-ai-worker --test process actual_local_inference -- --ignored --nocapture
RHYTHM_WORKER_TEST_BIN="$PWD/target/debug/rhythm-ai-worker" cargo test --locked -p rhythm intelligence::tests
```

The actual-model process test loads, queries status, performs real generation, unloads,
checks empty status, and shuts down. It checks protocol-only stdout and a nonempty model
response, not summary quality. The application-side test additionally exercises crash,
restart, failed-load/inference recovery, serialized concurrent health requests and cleanup.
Deterministic tests exercise protocol/version failures, bounded frames, invalid paths,
GGUF headers, input limits, request deadlines and cancellation cleanup.

Tauri app builds can overwrite `target/debug/rhythm-ai-worker` with the staged sidecar.
For exact-backend evaluations, copy a freshly built worker to a separate dedicated
path before building the app; explicitly stage the intended binary for native builds.
A Fresh Cargo message alone is not evidence of which binary remains at the shared path.

Verified after the upgrade on this macOS Apple Silicon host: existing
**Qwen3-0.6B-Q8_0.gguf, CPU**, with real generated output and successful lifecycle tests.
The same artifact also passed the explicit Metal-feature process test, reporting
`loaded_backend: "metal"` (3,629 ms loading and 872 ms generation in one debug sample).
The tiny baseline model's response was refusal-like, so this establishes real generation
and lifecycle behavior, **not useful summary quality**. The fallback policy is tested
with deterministic loader failures; an actual unavailable-GPU-to-CPU transition was
not forced. This baseline alone does not establish Qwen3.5 compatibility or quality.

Separately, the pinned **Qwen3.5 2B and 4B Q4_K_M** artifacts each completed one useful
synthetic Korean scholarship analysis through the actual application worker/controller/
validation/Store path on this macOS arm64 Metal setup. Both produced correct classification
and grounded deadline, persisted results, reused cache, isolated a new revision and read
cached output after database reopen with the worker stopped. Debug load/complete-analysis
observations were **8.121/83.528 s** for 2B and **12.820/171.156 s** for 4B; analysis includes
model integrity verification, not isolated generation. Post-load worker RSS samples were
**2,982,784 KiB** and **5,341,680 KiB**, not peak memory. Neither run establishes release
performance, broad Korean quality, native UI operation or OS-enforced offline inference.

The analysis layer now uses **prompt version 2 / stored schema 1**: generated facts select
source-owned passage IDs, the backend supplies exact evidence and derives explicit supported
dates, and one bounded validation repair attempt is allowed. Both final samples succeeded
on their first attempt. This application contract is distinct from the worker's generic
text protocol; see [`docs/AI_EVALUATION.md`](../../docs/AI_EVALUATION.md) for exact artifact
revisions/digests, output details, measurements and remaining gates.

Initial 4B CPU loading failed under then-current memory pressure before generation.
Qwen3.5 CPU/CUDA, 9B (not downloaded), Windows/Linux runtime, complete native flows and
release-optimized installers remain unverified. Do not infer those claims from Metal
compilation, baseline generation or the single-case application evidence.
