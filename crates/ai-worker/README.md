# Rhythm AI worker

A separate Rust executable. No model is loaded at startup, and no model is downloaded.
The default build contains the real `mistralrs = 0.8.1` CPU runtime. `--no-default-features`
produces a lifecycle-only worker whose status explicitly reports `runtime_available: false`;
this is not an inference implementation or a production replacement.

## Build and lifecycle test

From the repository root (current stable Rust, at least 1.90 for the locked dependencies):

```sh
cargo build -p rhythm-ai-worker
cargo test -p rhythm-ai-worker
cargo build -p rhythm-ai-worker --release
```

Optional hardware features:

```sh
cargo build -p rhythm-ai-worker --release --features metal
cargo build -p rhythm-ai-worker --release --features cuda
```

CPU is the portable default. Metal requires Apple's Metal Toolchain in addition to Xcode.
The initial macOS check failed in upstream `mistralrs-quant` and `mistralrs-paged-attn`
build scripts with `cannot execute tool 'metal' due to missing Metal Toolchain`.
No additional toolchain was installed. CUDA requires a compatible NVIDIA toolchain;
CUDA, Windows, Linux, and GPU inference have not been runtime verified.

## Protocol v1

One UTF-8 JSON request and response per line, max request frame 1 MiB. Requests are
processed serially. stdout is reserved for protocol. Diagnostics and upstream prints go
to stderr. Malformed requests return id `0`; valid requests preserve their numeric id.
Unsupported versions/methods return errors without terminating the worker. Oversized
frames terminate with a nonzero exit rather than accepting an unbounded allocation.
Closing stdin or sending `shutdown` exits the process; the parent must impose timeouts
and kill/reap the worker if inference becomes unresponsive.

```json
{"version":1,"id":1,"method":"ping"}
{"version":1,"id":2,"method":"status"}
{"version":1,"id":3,"method":"infer","params":{"model_path":"/absolute/path/model.gguf","prompt":"한국어로 짧게 인사해 주세요.","max_tokens":64}}
{"version":1,"id":4,"method":"shutdown"}
```

Response envelope:

```json
{"version":1,"id":2,"ok":true,"result":{"engine":"mistral.rs","runtime_available":true,"model":null}}
```

`ping` returns `{"healthy":true}`. `shutdown` returns `{"shutdown":true}`.
Successful `infer` returns `{"text":"<actual generated text>","model":"<canonical local path>"}`.
Errors return `{"version":1,"id":3,"ok":false,"error":"<Korean message>"}`.
No inference success is synthesized when a model is missing or incompatible.

## Actual local inference test

Supply an **already available, compatible GGUF** containing its tokenizer and chat template.
The initial verified artifact is **Qwen3-0.6B-Q8_0.gguf**, using the CPU backend on
macOS Apple Silicon. Other artifacts require their own compatibility test. Model weights are
not included. A model without an embedded usable chat template may require companion
metadata; it is not automatically fetched. Paths must be local existing `.gguf` files,
not Hugging Face repository identifiers. The worker canonicalizes the path and passes
the existing directory to upstream's local-file loader; no remote tokenizer ID is set,
and no authentication token is read (`TokenSource::None`).

Run the opt-in test on macOS/Linux:

```sh
RHYTHM_TEST_MODEL=/absolute/path/model.gguf cargo test -p rhythm-ai-worker --test process actual_local_inference -- --ignored --nocapture
```

On PowerShell:

```powershell
$env:RHYTHM_TEST_MODEL = 'C:\Models\model.gguf'
cargo test -p rhythm-ai-worker --test process actual_local_inference -- --ignored --nocapture
```

The test invokes the real executable and requires a nonempty actual generated response,
protocol-only stdout, and clean shutdown after loading the model. It does not download
models. This test **passed on macOS Apple Silicon CPU** with an existing
Qwen3-0.6B-Q8_0.gguf, producing a real greeting. Requests disable extended thinking
because this foundation returns final text rather than reasoning streams.

Do not enable upstream's `ring` feature for ordinary single-machine builds: it selects
distributed inference and caused an actual-model test to fail without `RING_CONFIG`.
The default intentionally excludes it. Planned Qwen3.5 2B/4B/9B model support is
**not claimed**: the pinned upstream GGUF loader must support the particular architecture
and quantization. Verify compatibility against a specific local artifact before listing
it as supported.

The worker retains at most one loaded model. It drops the old model before switching,
and the OS reclaims the worker's memory on process exit. There is no streaming,
cancellation, model installation, automatic downloads, tool execution, or network API.

## Upstream references

- [mistralrs 0.8.1](https://crates.io/crates/mistralrs/0.8.1) (MIT, Rust 1.88 minimum).
- [Official local GGUF example](https://github.com/EricLBuehler/mistral.rs/blob/master/mistralrs/examples/getting_started/gguf_locally/main.rs).
- [GGUF builder API](https://docs.rs/mistralrs/0.8.1/mistralrs/struct.GgufModelBuilder.html).

The implementation is checked against the published 0.8.1 API, not assumed to track
upstream master. Its GGUF parser contains unconditional `println!` calls, so the worker
uses `filedescriptor`'s Unix/Windows abstraction to preserve a private stdout pipe and
redirect process stdout to stderr before initializing the runtime.
