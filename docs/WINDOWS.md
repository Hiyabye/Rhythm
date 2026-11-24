# Windows 11 x64

Rhythm supports Windows 11 x64 alongside macOS Apple Silicon. The port came from contributor PR #1; what has actually been observed on Windows is in [Windows validation](WINDOWS_VALIDATION.md). A working build environment does not verify the installer or the app's runtime behavior.

## What ships

- A per-user NSIS installer (`installMode: currentUser`, custom `scripts/installer-template.nsi` and `installer-hooks.nsh`), unsigned. SmartScreen warnings are expected. Release builds also sign the installer for the Tauri updater (`.sig`). In-app updates run it in passive mode (`/P /UPDATE /R`): the custom template skips the uninstall and desktop-shortcut steps and relaunches the app, and Rhythm stops `llama-server` first so its executable is not locked. It was observed on one PC under a validation identifier with a loopback manifest (see [Windows validation](WINDOWS_VALIDATION.md#in-app-update)); the production identifier and GitHub endpoint were not exercised. Uninstall always keeps user data and models.
- The installer puts the app in `%LOCALAPPDATA%\Programs\Rhythm`. A missing WebView2 Runtime is downloaded from Microsoft during installation (`webviewInstallMode: downloadBootstrapper`, silent), so a first install on such a PC needs internet.
- The pinned upstream llama.cpp CPU build as resources in `llama\cpu\`: `llama-server.exe`, its DLLs and a copy of the VC runtime DLLs it imports. Releases before 2.0 also shipped a Vulkan build in `llama\vulkan\`; the installer deletes that folder on upgrade (#178).
- App-local Microsoft VC runtime DLLs (`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`), copied from the official Visual Studio redistributable with signature and hash checks by `scripts/stage-vc-runtime.ps1`. The app's import scan found an `MSVCP140.dll` dependency, so static CRT linking alone is not relied on.
- Windows-only config overrides in `src-tauri/tauri.windows.conf.json` (merge-patched over `tauri.conf.json`; arrays replace).

## Build environment

- Windows 11 x64, Node.js 22.12+ and npm, Microsoft Edge WebView2 Runtime, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
- Visual Studio Build Tools with x64 MSVC and the Windows 11 SDK (observed: Build Tools 17.14.44, MSVC 14.44.35207, SDK 10.0.26100.0).
- Stable Rust `x86_64-pc-windows-msvc`, 1.90+.
- No LLVM or CMake: `scripts/stage-llama.mjs` downloads the pinned llama.cpp build (network access) and checks their size and SHA-256.
- Roughly 10 GiB free on C: and 30 GiB on D:. Dependencies, model weights and build outputs add more.

### Project-local toolchain (optional)

`scripts/setup-windows.ps1` checks the OS, free space, Node/npm and WebView2, then installs Visual Studio Build Tools and Rust into a `.build-tools` folder next to the checkout (the repository must be on D:). It downloads only from Microsoft and rust-lang.org over HTTPS, checks the Visual Studio bootstrapper's Authenticode signature and the rustup SHA-256, never reboots, and stops instead of overwriting an unexpected installation. Installing Build Tools needs an administrator PowerShell. It installs no GPU tools; none are needed.

```powershell
& .\scripts\setup-windows.ps1     # once
. .\scripts\windows-env.ps1       # in every build shell; changes no persistent PATH
npm ci
npm run desktop
```

`windows-env.ps1` puts Cargo output under `..\.build-tools\target` and exports the current PowerShell as `RHYTHM_POWERSHELL`. Without it, put Cargo and the MSVC tools on PATH yourself:

```powershell
$env:RHYTHM_POWERSHELL = Join-Path $PSHOME 'pwsh.exe'   # when building from PowerShell 7
npm run desktop:build
```

## Build behavior

- `npm run desktop` and `npm run desktop:build` delegate to `scripts/desktop-windows.mjs`. It stages the VC runtime first (so missing tools fail early), then the pinned llama.cpp CPU build (`scripts/stage-llama.mjs`, which copies those VC runtime DLLs beside it), then runs Tauri.
- VC runtime staging runs under `RHYTHM_POWERSHELL` (default `powershell.exe`). The script never changes execution policy and never retries under another host.
- llama.cpp archives are extracted with Windows' own `System32\tar.exe`, because Git's GNU `tar` earlier on PATH cannot read zip files.
- A release build writes only `Rhythm_<version>_x64-setup.exe` and its `.sha256` to `target/release/distribution/`, plus `<exe>.sig` when `TAURI_SIGNING_PRIVATE_KEY` and `RHYTHM_UPDATER_PUBKEY` are both set.
- `target-cpu=native` is rejected because installers must run on other PCs.

## Embedding server

- The classifier runs the CPU build only (#178 removed the Vulkan build and the execution-mode setting).
- The server listens on 127.0.0.1 on a port the OS chooses, and every request needs a random key written for that launch only; the key file is deleted once the server is ready.

To check a real model on this PC, stage the build and run the opt-in evaluation with a model in an isolated folder (see [AI evaluation](AI_EVALUATION.md#running-evaluations)); it never touches app data.

## Restore checks

Windows restore requires a local fixed NTFS volume and refuses Cloud Files sync roots. The check calls the real Cloud Files API; `0x80070186` (not under a sync root) is the only accepted result. To check your own temp folder without changing anything:

```powershell
. .\scripts\windows-env.ps1
& .\scripts\prepare-windows-restore-ci.ps1 -CheckOnly
```

On GitHub-hosted runners only, the script may also enable and attach the `CldFlt` filter before checking again. Restricted tokens can return access denied; any result other than `0x80070186` refuses restore.
