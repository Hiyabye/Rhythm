# Windows 11 x64

Rhythm supports Windows 11 x64 alongside macOS Apple Silicon. The port came from contributor PR #1; what has actually been observed on Windows is in [Windows validation](WINDOWS_VALIDATION.md). A working build environment does not verify the installer or the app's runtime behavior.

## What ships

- A per-user NSIS installer (`installMode: currentUser`, custom `scripts/installer-template.nsi` and `installer-hooks.nsh`), unsigned. SmartScreen warnings are expected. Updates are manual reinstallation; uninstall always keeps user data and models.
- Two AI workers: the CPU worker (`rhythm-ai-worker.exe`, mistral.rs) and the NVIDIA GPU worker (`rhythm-ai-worker-gpu.exe`, llama.cpp over Vulkan). Installers always contain both.
- App-local Microsoft VC runtime DLLs (`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`), copied from the official Visual Studio redistributable with signature and hash checks by `scripts/stage-vc-runtime.ps1`. A release-binary import scan found an `MSVCP140.dll` dependency, so static CRT linking alone is not relied on. `vulkan-1.dll` and the Vulkan driver come from the user's graphics driver and are never bundled.
- Windows-only config overrides in `src-tauri/tauri.windows.conf.json` (merge-patched over `tauri.conf.json`; arrays replace).

## Build environment

- Windows 11 x64, Node.js 22.12+ and npm, Microsoft Edge WebView2 Runtime, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
- Visual Studio Build Tools with x64 MSVC and the Windows 11 SDK (observed: Build Tools 17.14.44, MSVC 14.44.35207, SDK 10.0.26100.0).
- Stable Rust `x86_64-pc-windows-msvc`. The locked mistral.rs needs Rust 1.94+.
- For the GPU worker: Vulkan SDK 1.4.363.0, LLVM 20.1.8 (for libclang), CMake and Ninja (the Ninja bundled with the Visual Studio C++ CMake tools works from the MSVC developer environment).
- Roughly 10 GiB free on C: and 30 GiB on D:. Dependencies, model weights and build outputs add more.

Pinned GPU tool installers (CI downloads and checks the same files in `.github/actions/windows-build-tools`):

| File | SHA-256 |
| --- | --- |
| [Vulkan SDK 1.4.363.0](https://sdk.lunarg.com/sdk/download/1.4.363.0/windows/vulkansdk-windows-X64-1.4.363.0.exe) | `94a82d378f7a5e3e54c9db7d2fb7016af136e14ac0a18dbf0f2f67a36352d141` |
| [LLVM 20.1.8](https://github.com/llvm/llvm-project/releases/download/llvmorg-20.1.8/LLVM-20.1.8-win64.exe) | `3197846a2b19063687dd56e93e34cd941e3548d907f23a6131571321bdf9fe7b` |

The Vulkan SDK can be installed with `copy_only=1` into a separate folder without registry or system PATH changes. The SDK is a build tool only; users do not need it.

### Project-local toolchain (optional)

`scripts/setup-windows.ps1` checks the OS, free space, Node/npm and WebView2, then installs Visual Studio Build Tools and Rust into a `.build-tools` folder next to the checkout (the repository must be on D:). It downloads only from Microsoft and rust-lang.org over HTTPS, checks the Visual Studio bootstrapper's Authenticode signature and the rustup SHA-256, never reboots, and stops instead of overwriting an unexpected installation. Installing Build Tools needs an administrator PowerShell. It does not install the GPU tools; place the Vulkan SDK at `.build-tools/vulkan/1.4.363.0` and LLVM at `.build-tools/llvm` and `windows-env.ps1` picks them up.

```powershell
& .\scripts\setup-windows.ps1     # once
. .\scripts\windows-env.ps1       # in every build shell; changes no persistent PATH
npm ci
npm run desktop
```

`windows-env.ps1` puts Cargo output under `..\.build-tools\target`, the GPU worker under the short `..\.g`, and exports the current PowerShell as `RHYTHM_POWERSHELL`. Without it, set the tools yourself:

```powershell
$env:VULKAN_SDK = 'D:\BuildTools\VulkanSDK\1.4.363.0'
$env:LIBCLANG_PATH = 'D:\BuildTools\LLVM\bin'
$env:RHYTHM_GPU_TARGET_DIR = 'D:\rhythm-gpu'
$env:PATH = "$env:VULKAN_SDK\Bin;$env:PATH"
$env:RHYTHM_POWERSHELL = Join-Path $PSHOME 'pwsh.exe'   # when building from PowerShell 7
npm run desktop:build
```

## Build behavior

- `npm run desktop` and `npm run desktop:build` delegate to `scripts/desktop-windows.mjs`. It stages the VC runtime first (so missing tools fail before long compiles), then builds both workers and runs Tauri.
- VC runtime staging runs under `RHYTHM_POWERSHELL` (default `powershell.exe`). The script never changes execution policy and never retries under another host.
- The GPU worker builds with `--no-default-features --features gpu-inference` into `RHYTHM_GPU_TARGET_DIR` (default: `CARGO_TARGET_DIR` plus `-gpu`). That path must be at most 32 characters, because Vulkan shader generation hits nested-path limits, and must differ from the CPU target directory.
- A release build writes only `Rhythm_<version>_x64-setup.exe` and its `.sha256` to `target/release/distribution/`.
- `RHYTHM_AI_BACKEND=cpu` skips the GPU worker and excludes its sidecar. It is allowed only for `dev` or `build --no-bundle`; installers need both workers. It is a build option, not the in-app execution mode.
- `target-cpu=native` is rejected because installers must run on other PCs.

## GPU worker

- `llama-cpp-2`/`llama-cpp-sys-2` pinned to `0.1.160` with Vulkan; llama.cpp is linked statically. It speaks the same JSON-lines protocol as the CPU worker and opens no port.
- Targets NVIDIA GTX 10/16 and RTX 20/30/40/50. Only a GTX 1650 has been tested. The pinned mistral.rs CUDA kernels need newer GPUs than Pascal, which is why the GPU path is Vulkan.
- Accepts only an NVIDIA Vulkan device and reports a GPU load only after it observes offloaded layers and an allocated GPU buffer. Some layers may stay on the CPU; it tries to leave 512 MiB of VRAM free, which does not prevent contention with other programs.
- Context is 8192 tokens (input plus output). An input that does not fit is rejected with `토큰 한도를 넘습니다`.
- **AI 실행 장치** (`auto`/`cpu`/`gpu`, schema 8) picks the sidecar. `gpu` reports errors. `auto` switches to CPU for the session after a device, load or protocol failure; a context-overflow rejection retries only that request on CPU. Explicit CPU and GPU results have separate cache identities; `auto` accepts either, and every result records its `loaded_backend`.

Run the GPU worker against a real model without touching app data (writes a new report file only):

```powershell
node .\scripts\validate-gpu.mjs --worker 'D:\rhythm-gpu\release\rhythm-ai-worker.exe' --model 'D:\models\Qwen3.5-2B-Q4_K_M.gguf' --report 'D:\validation\gpu-2b.json'
```

## Restore checks

Windows restore requires a local fixed NTFS volume and refuses Cloud Files sync roots. The check calls the real Cloud Files API; `0x80070186` (not under a sync root) is the only accepted result. To check your own temp folder without changing anything:

```powershell
. .\scripts\windows-env.ps1
& .\scripts\prepare-windows-restore-ci.ps1 -CheckOnly
```

On GitHub-hosted runners only, the script may also enable and attach the `CldFlt` filter before checking again. Restricted tokens can return access denied; any result other than `0x80070186` refuses restore.
