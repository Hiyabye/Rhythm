# Rhythm AI worker

Rhythm bundles an isolated Rust executable for local inference. It starts without
weights, retains at most one model and opens no server or network port. Stdout is
exclusively protocol v1 JSON Lines; diagnostics, including upstream output, go to
stderr. The worker shares the application version and installer. It is not an
independently updated service.

## Runtime and builds

All inference builds use the same llama.cpp implementation through
`llama-cpp-2`/`llama-cpp-sys-2`, pinned to `0.1.160` by the workspace lockfile.
The supported products are macOS Apple Silicon and Windows 11 x64.

| Cargo features | Compiled backend | Shipped executable |
| --- | --- | --- |
| Default (`inference`) | CPU | `rhythm-ai-worker` |
| `metal` | Apple Metal, with CPU fallback | macOS `rhythm-ai-worker` |
| `--no-default-features --features gpu-inference` | NVIDIA Vulkan | Windows `rhythm-ai-worker-gpu.exe` |
| `--no-default-features` | No inference, lifecycle tests only | Not a production worker |

Metal and Vulkan cannot be enabled together. Windows installers include separate CPU
and Vulkan executables, so CPU startup does not depend on a Vulkan driver. CUDA is not a feature.
The common engine in `src/inference.rs` owns GGUF loading, chat formatting, constrained decoding and generation;
backend selection does not duplicate those contracts.

`npm run worker:build` prepares release workers independently of the app; add `-- --debug`
for debug workers. Desktop builds reuse only matching manifest, SHA-256 and actual status
identities, rebuilding when those checks fail. See `AGENTS.md` for that build contract.
Direct Cargo commands remain useful for focused runtime work:

```sh
cargo build --locked -p rhythm-ai-worker
cargo test --locked -p rhythm-ai-worker
cargo build --locked --release -p rhythm-ai-worker --features metal
```

Windows Vulkan builds need a short, separate target directory and the toolchain described
in the [Windows guide](../../docs/WINDOWS.md):

```powershell
$env:LLAMA_BUILD_SHARED_LIBS = '0'
$env:LLAMA_STATIC_CRT = '1'
cargo build --locked --release -p rhythm-ai-worker --no-default-features --features gpu-inference --target-dir D:\rhythm-gpu
```

Desktop scripts select Metal on Apple Silicon when the existing Metal compiler probe
succeeds, otherwise CPU. Explicit `RHYTHM_AI_BACKEND=metal` fails if that tooling is
missing. Build scripts do not install model weights or inference services.

`build_backend` describes the compiled backend, while `loaded_backend` reports the
actual model placement and is null when unloaded. A failed Metal load retries once on
CPU within the parent request deadline. Vulkan accepts only NVIDIA devices and reports
GPU loading only after observing offloaded layers and an allocated GPU buffer. It
reserves an estimated 512 MiB of VRAM; this cannot prevent contention with other apps.
Windows fallback is owned by the app: `auto` switches to its CPU executable after a
device/load/protocol failure, while explicit `gpu` returns the error.

NVIDIA GTX 10/16 and RTX 20/30/40/50 are targets, not a list of verified hardware.
Current artifact/backend evidence belongs in [AI evaluation](../../docs/AI_EVALUATION.md);
previous mistral.rs results do not verify this runtime.

## Protocol v1

Requests are serial, version- and ID-matched UTF-8 JSON Lines, bounded to 1 MiB per frame.
Unknown versions/methods return errors; malformed JSON returns ID 0; oversized frames
terminate the worker. Closing stdin or sending `shutdown` ends it.

```json
{"version":1,"id":1,"method":"ping"}
{"version":1,"id":2,"method":"status"}
{"version":1,"id":3,"method":"load","params":{"model_path":"/absolute/model.gguf"}}
{"version":1,"id":4,"method":"infer","params":{"model_path":"/absolute/model.gguf","prompt":"분석할 문서","max_tokens":1024,"output_mode":"analysis_json_v1"}}
{"version":1,"id":5,"method":"unload"}
{"version":1,"id":6,"method":"shutdown"}
```

Status reports `engine: "llama.cpp"`, `build_id`, `runtime_available`, compiled/loaded backend,
model and load timing. `infer` loads or switches on demand. Responses retain the existing
`text`, `model`, load/reuse/timing, `finish_reason` and optional `usage` fields.
`load_ms` is the last successful model load; `request_load_ms` and `model_reused`
describe the current request. Missing optional legacy metrics remain unknown.
Model metadata, compiled acceleration and inference timings are not quality guarantees.

The optional `analysis_json_v1` capability enables a fixed JSON schema compiled to
llama.cpp grammar. Callers cannot supply arbitrary grammars. Absent/null output mode
keeps ordinary text for setup checks; unknown modes fail. The schema allows five required
fields, eight categories and at most six facts with exactly `kind`, `label`, `value`
and nonnegative `evidence_id`. It must match the app's analysis ingress. Grammar does
not establish source grounding or correct meaning; validators and one bounded repair
remain app responsibilities.

## Model and generation boundaries

Only canonical local `.gguf` files pass to the engine after GGUF v2/v3 header checks.
The app separately verifies the full artifact SHA-256. The runtime reads embedded
tokenizer/chat metadata; incomplete artifacts fail rather than fetching replacements.
No vision projector, attachments, remote tokenizer, tools, search or command execution
are enabled. Chat control delimiters in source text are neutralized, thinking is disabled
and a fixed system instruction treats source text as untrusted data.

The old model is dropped before switching. Reuse requires the same canonical path,
size and modification time; this is a reload check, not an integrity substitute.
The app unloads changed external artifacts before re-registration can claim a new hash.

Inputs are nonempty and at most 24 KiB UTF-8; outputs are 1–4096 tokens (default 128).
CPU/Metal context is 32,768 tokens and Vulkan context is 8192, including input and output.
Oversized inputs fail without silent truncation. Vulkan overflow includes
`토큰 한도를 넘습니다`, allowing Windows `auto` to retry only that request on CPU.
Generation uses greedy sampling, one sequence and a fresh inference context per request.

## Lifecycle and verification

The app owns the executable, pipes and child process. Deadlines are 8 seconds for lifecycle
requests, 180 seconds for loading and 600 seconds for inference. Invalid framing or
responses and failed load/inference reap the worker; the next request can start a new one.
Cancellation is process cancellation: stop and reap before reusing a pipe. App-level
unload restarts an empty worker to reclaim OS memory; direct protocol unload only drops
the model. Shutdown requests a clean exit before forced termination.

Routine tests do not download weights. With an existing artifact and explicit test scope:

```sh
node scripts/validate-worker.mjs --backend cpu --worker /absolute/rhythm-ai-worker --model /absolute/model.gguf --report /absolute/new-report.json
RHYTHM_TEST_MODEL=/absolute/model.gguf cargo test --locked -p rhythm-ai-worker --test process actual_local_inference -- --ignored --nocapture
RHYTHM_WORKER_TEST_BIN=/absolute/rhythm-ai-worker cargo test --locked -p rhythm intelligence::tests
```

The Node helper accepts `cpu`, `vulkan` or `metal` and verifies actual backend placement,
structured output and owned-process cancellation/recovery. Metal CPU fallback is rejected
unless `--allow-cpu-fallback` is explicit; that result is labeled separately. The legacy
`validate-gpu.mjs` entry point fixes the backend to Vulkan. Semantic observations do not
decide the integration pass. See [evaluation commands](../../docs/AI_EVALUATION.md#running-evaluations).

The real-model process test covers load, status, generation, unload and shutdown; a
nonempty response proves generation, not summary accuracy. App tests cover process
failure/restart and cleanup. Build the exact backend before testing and preserve a
separate named executable if later app builds could overwrite the shared Cargo output.
Never report an ignored or skipped real-worker test as inference verification.

Current runtime measurements, historical evidence boundaries and evaluation commands
are recorded once in [AI evaluation](../../docs/AI_EVALUATION.md).
