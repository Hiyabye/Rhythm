# Rhythm AI worker

Rhythm bundles a separate Rust executable, `rhythm-ai-worker`, for local inference.
It starts without weights, retains at most one model, and uses no inference server or
network port. Diagnostic output (including upstream stdout prints) is redirected to
stderr; stdout carries only versioned JSON Lines IPC.

## Runtime and compatibility

Current application/worker version is **0.4.0**. The worker protocol remains v1;
application prompt versioning and stored-result schemas are separate contracts.

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

Analysis additionally negotiates the optional `analysis_json_v1` status capability and
sends `"output_mode":"analysis_json_v1"` only to a supporting worker. Absent/null mode
keeps ordinary text generation, including setup health checks. Unknown modes fail;
callers cannot supply arbitrary grammars. Legacy workers remain usable for ordinary
inference, but constrained analysis returns rebuild guidance without sending an unknown
parameter or poisoning the existing pipe. Malformed supplied capability values fail
validation and reap the child.

The analysis mode uses the pinned runtime's existing `Constraint::JsonSchema` API with
a static closed schema: five required output fields, the eight categories, qualitative
uncertainty, and at most six facts containing exactly `kind`, `label`, `value` and
nonnegative `evidence_id`. Keep this schema aligned with application generation ingress.
The mode explicitly stops at `<|endoftext|>`: a real 2B trial otherwise produced complete
JSON followed by repeated grammar-EOS tokens until the token limit. This is termination
handling, not post-hoc trimming of invalid output. Output-limit/runtime failures can
still prevent a complete JSON result. Source/date/time/link and duplicate validators,
one bounded repair and cancellation remain application responsibilities. Grammar does
not prove correct categories, current deadlines or preserved exclusions. See the
[bounded A/B results](../../docs/AI_EVALUATION.md#bounded-schema-decoding-experiment).

Model paths are canonical local `.gguf` files, with GGUF v2/v3 magic/version validation
before passing them to mistral.rs. Full architecture/tensor validation occurs in the
runtime. The previous model is dropped before another is loaded. Reuse requires the
same canonical path, file size and modification timestamp. Changes force a reload;
this metadata check is not an integrity guarantee. The application independently
checks the artifact SHA-256 and unloads on external-file re-registration.

```json
{"version":1,"id":2,"ok":true,"result":{"engine":"mistral.rs","runtime_available":true,"model":null,"load_ms":0,"build_backend":"cpu","loaded_backend":null,"analysis_json_v1":true}}
{"version":1,"id":4,"ok":true,"result":{"text":"실제 생성된 응답","model":"/absolute/path/model.gguf","load_ms":123,"request_load_ms":0,"model_reused":true,"loaded_backend":"cpu","input_validation_ms":0,"request_preparation_ms":1,"generation_ms":456,"finish_reason":"stop","usage":{"completion_tokens":10}}}
{"version":1,"id":4,"ok":false,"error":"한국어 오류 메시지"}
```

The `usage` example is abbreviated: the runtime returns prompt/completion counts and
its timing/throughput fields. The parent preserves these as typed optional diagnostics
through `infer_detailed` and capability-negotiated `infer_analysis`; `infer` remains a text-returning compatibility wrapper.
`load_ms` is the historical last successful load duration, not this request's load cost.
`request_load_ms` and `model_reused` distinguish loading from reuse (zero request load
when reused). Input validation, request preparation, generation wall time, finish reason
and upstream engine usage are separate fields; engine timings are not end-to-end app
latency. Missing legacy metrics remain unknown, while malformed supplied metrics fail
and reap the worker. These are request observations, not performance guarantees.
`load`/`unload` return status; `ping` returns `healthy`, and `shutdown` returns `shutdown`.

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
request reloads once; normal completed requests reuse the worker/model. The application's
in-memory operation owner exposes load/setup/register phases and cancellation across
Settings remounts. A cancelled blocking verifier is joined before releasing admission;
cancellation during serial IPC stops/reaps the child. Setup completion is persisted only
after its real inference succeeds. Application
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

In the historical v0.3.0 baseline, the pinned **Qwen3.5 2B and 4B Q4_K_M** artifacts each
completed one useful synthetic Korean scholarship analysis through the actual worker/controller/
validation/Store path on this macOS arm64 Metal setup. Both produced correct classification
and grounded deadline, persisted results, reused cache, isolated a new revision and read
cached output after database reopen with the worker stopped. Debug load/complete-analysis
observations were **8.121/83.528 s** for 2B and **12.820/171.156 s** for 4B; analysis includes
model integrity verification, not isolated generation. Post-load worker RSS samples were
**2,982,784 KiB** and **5,341,680 KiB**, not peak memory. Neither run establishes release
performance, broad Korean quality, native UI operation or OS-enforced offline inference.

Those baseline samples used **prompt version 2 / stored schema 1** and succeeded on their
first attempt. Current v0.3.1 uses **prompt 3 / stored schema 1** with explicit `{id,text}`
source objects. Generation ingress requires only `kind`, `label`, `value`, `evidence_id`
per fact; the backend supplies exact evidence and conservatively derived dates/uncertainty.
Literal date/time validation and one bounded repair remain. Long input selects exact
head/tail/important ranges within 18 KiB of encoded source and a 24 KiB prompt. Selector
v4 deduplicates exact repeated passages only over budget and separates structural table
rows; it is not full-document understanding. Full model-file hashing remains mandatory;
the parent enables approved `sha2` assembly only on macOS aarch64.

The current local checkpoint passed **175 workspace Rust tests** (eight opt-in tests
ignored), strict workspace Clippy, 86 mocked-IPC browser checks and five POSIX desktop
checks, and built a debug macOS bundle. Separate isolated native 2B/Metal checks covered
load/remount/cancel, setup cancellation during verification, successful real setup and
OS-network-denied prompt-v3 regeneration. These are functional checks, not broad quality
or release-performance guarantees. An initial 18-synthetic/six-public-archive case campaign
per model completed, followed by an eight-synthetic/three-archive-per-model decoding A/B
experiment. It improved completed jobs from 17/22 to 20/22 with similar single-case timings,
so analysis now uses the negotiated constrained mode. Semantic mistakes remain, and the
full 18-case campaign has not been repeated under the final default; neither grammar nor
prompt changes prove those mistakes eliminated.

These application contracts are distinct from the worker's generic text protocol; see
[`docs/AI_EVALUATION.md`](../../docs/AI_EVALUATION.md) for exact artifact identities,
measurements, final campaign outcomes and remaining gates. Transient diagnostics retain
bounded timing/count reports without document or generated text; they are not persisted.

Initial 4B CPU loading failed under then-current memory pressure before generation.
Qwen3.5 CPU/CUDA, 9B (not downloaded), Windows/Linux runtime, complete native-flow coverage
and release-optimized installer distribution remain unverified. Do not infer those claims
from Metal compilation, baseline generation or the single-case application evidence.
