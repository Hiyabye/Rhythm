# Rhythm AI worker

Rhythm uses a separate Rust process for local inference. The worker starts without
weights, keeps at most one model, and communicates with the application over bounded
UTF-8 JSON Lines pipes. It opens no inference server or network port. Diagnostic output
is redirected away from the protocol pipe.

This Windows port builds two independent executables from this crate. The default
CPU worker uses the commit-pinned mistral.rs dependency. The NVIDIA GPU worker uses
`llama-cpp-2` and `llama-cpp-sys-2`, both pinned to `0.1.160`, with Vulkan. CPU and GPU
inference features are mutually exclusive; desktop packaging includes both workers
so the application can select one and retain a CPU fallback. Metal and CUDA are not
product build options. Both workers load a local GGUF file without a remote tokenizer
or tool, search, shell, or execution capabilities. The application verifies model size,
SHA-256, and GGUF metadata before use.

The GPU target is NVIDIA GTX 10/16 and RTX 20/30/40/50, not a claim that every device
has been tested. The worker requires an NVIDIA Vulkan device and observed model-layer
offload plus an allocated GPU model buffer before reporting a successful GPU load.
It also checks context allocation; partial offload may leave layers on the CPU.
Compiled runtime availability alone does not establish GPU availability or generation.
See the [Windows GPU guide](../../docs/WINDOWS_GPU.md) for driver/build requirements
and actual hardware evidence.

The model catalog permits users to install Qwen3.5 2B and 4B for Windows CPU
validation. Previous macOS Metal results are historical evidence, not Windows CPU
verification. The 9B catalog entry remains unavailable until separately evaluated.
The application must remain usable if the worker or model is unavailable.

## Build and test

Use the desktop helper to build and stage both Windows sidecars. The GPU build also
requires Vulkan SDK (`VULKAN_SDK`), libclang (`LIBCLANG_PATH`), CMake and Ninja in the
MSVC development environment; these development tools are not installed with the app.
The helper uses an independent short `RHYTHM_GPU_TARGET_DIR` and static native
libraries. See the GPU guide for reproducible dependency setup. Build/test the CPU
worker directly with:

```powershell
cargo build --locked -p rhythm-ai-worker --release
cargo test --locked -p rhythm-ai-worker
```

For isolated GPU worker development, after setting up those dependencies:

```powershell
$env:CMAKE_GENERATOR = 'Ninja'
$env:LLAMA_BUILD_SHARED_LIBS = '0'
$env:LLAMA_STATIC_CRT = '1'
cargo build --locked -p rhythm-ai-worker --release --no-default-features --features gpu-inference --target-dir D:\rhythm-gpu
cargo test --locked -p rhythm-ai-worker --release --no-default-features --features gpu-inference --target-dir D:\rhythm-gpu
```

Keep the GPU target directory distinct from the CPU directory and at most 32
characters long for Windows shader compilation. Do not use `target-cpu=native`.
The GPU executable still has the crate's filename until the desktop helper stages it
as `rhythm-ai-worker-gpu-<host-triple>.exe`.

The default build includes CPU inference. `--no-default-features` alone creates a lifecycle-only
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

To test actual GPU inference, add `--release --no-default-features --features
gpu-inference --target-dir D:\rhythm-gpu` before `--test process`, using the same native
build environment as above. The GPU variant additionally requires reported NVIDIA
device identity and positive GPU-layer offload. Tests must still collect real output
before claiming actual inference succeeded; ordinary unit/process tests do not load
weights.

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

The GPU worker accepts Qwen3/Qwen3.5 GGUF text models and fixes the context budget to
8,192 input-plus-output tokens with one sequence. It rejects overflow rather than
silently truncating source material, attempts to reserve 512 MiB of GPU memory, and
fails when no layers can be offloaded. This margin does not guarantee allocation
success when other applications compete for memory. The model's ChatML template and
explicit no-thinking assistant prefix are used for generation; analysis uses the
same `analysis_json_v1` schema as the CPU worker through a native grammar sampler.

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
