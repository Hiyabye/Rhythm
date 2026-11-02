# Rhythm AI worker

Rhythm uses a separate Rust process for local inference. The worker starts without
weights, keeps at most one model, and communicates with the application over bounded
UTF-8 JSON Lines pipes. It opens no inference server or network port. Diagnostic output
is redirected away from the protocol pipe.

This Windows port builds a **CPU-only** worker. Metal and CUDA are not product build
options. The worker uses the commit-pinned mistral.rs dependency in `Cargo.toml` and
loads a local GGUF file without a remote tokenizer or tool, search, shell, or execution
capabilities. The application verifies model size, SHA-256, and GGUF metadata before
use. A loaded model is reported as CPU only when the runtime reports that device.

The model catalog permits users to install Qwen3.5 2B and 4B for Windows CPU
validation. Previous macOS Metal results are historical evidence, not Windows CPU
verification. The 9B catalog entry remains unavailable until separately evaluated.
The application must remain usable if the worker or model is unavailable.

## Build and test

Use the desktop helper to build and stage the Windows sidecar, or build it directly:

```powershell
cargo build --locked -p rhythm-ai-worker --release
cargo test --locked -p rhythm-ai-worker
```

The default build includes inference. `--no-default-features` creates a lifecycle-only
worker for protocol tests; it reports `runtime_available: false` and cannot perform
inference. It is not a production substitute. Builds require a native Windows x64
MSVC host. Release builds link the VC runtime statically while using the Windows UCRT;
the actual distribution binary's dependencies must still be checked after building.
These commands do not download model
weights. When explicitly validating a model that is already installed, run the ignored
actual-model process test with `RHYTHM_TEST_MODEL` set to its full local GGUF path:

```powershell
$env:RHYTHM_TEST_MODEL = 'C:\path\to\model.gguf'
cargo test --locked -p rhythm-ai-worker --test process actual_local_inference -- --ignored --nocapture
```

That test checks real generation and the process protocol, not summary accuracy. For
the app-side real-worker lifecycle test set `RHYTHM_WORKER_TEST_BIN` to the full path
of the built executable. Without that variable the integration test skips. The
application's optional full analysis evaluations require additional explicit inputs;
see `docs/AI_EVALUATION.md`. Tests must use isolated application data.

## Protocol and lifecycle

Protocol v1 accepts one request per line, with a 1 MiB frame limit. It supports
`ping`, `status`, `load`, `unload`, `infer`, and `shutdown`. Inputs are bounded to
24 KiB UTF-8 and 1–4096 output tokens. `analysis_json_v1` is capability-negotiated;
its closed generation schema must match the application validator. JSON shape and
literal evidence checks do not prove semantic correctness.

The application owns the child and pipes, enforces timeouts, and reaps the child on
cancel or failure before reusing the connection. Unloading restarts the process to
release model allocations. A subsequent request can start a fresh worker. Windows
assigns each worker to a non-inheritable Job Object with kill-on-close so it exits if
the application process is forcibly terminated. Assignment happens immediately after
the worker is spawned; a parent crash in the short interval before assignment remains
an uncovered race. A failure to create or assign the job prevents normal worker use.
Native process behavior, real-model output, memory use, and performance must be
verified on the target machine; compiling or passing mocked tests does not establish
those results.
