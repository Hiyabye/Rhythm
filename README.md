# Rhythm

**내 기기에 머무는, 나만의 하루를 위한 커맨드 센터.**

Rhythm is a Korean-first desktop application for organizing personal information. Its long-term goal is to bring useful announcements, opportunities, and deadlines together and make sense of them with local AI—not a cloud chatbot.

## Principles

- **Local-first:** SQLite stores settings and interests on your computer. No cloud account, hosted database, telemetry, or cloud inference is required.
- **AI-native:** a bundled Rust worker contains the local inference runtime. No Ollama, Python environment, or separately installed inference server is needed.
- **User-controlled:** interests and extension preferences are explicit. Disabling an extension does not delete its data.
- **Quiet by design:** a restrained Korean interface, Pretendard typography, and light, dark, and system themes.

## Current status — 0.1.0 foundation

The five main sections—오늘, 받은 소식, 탐색, 보관함, 설정—are present. Settings persist theme, energy profile, interests, and extension preferences locally. The Sogang Notices extension is registered but **does not collect notices yet**. The inbox, briefing, search, and saved-item pages are intentional empty states, not populated demonstrations.

The separate `rhythm-ai-worker` contains mistral.rs 0.8.1 and supports health, status, shutdown, and an explicit local-model inference request. No model is bundled or downloaded at launch. Model installation, automatic summaries, notifications, background operation, and real document search are not available in this release. Qwen3.5 2B/4B/9B are planned, not verified model support.

## Run locally

You need:

- Node.js **22.12 or newer** and npm.
- A current stable Rust toolchain, **at least 1.90**, with `cargo` and `rustc` on `PATH`.
- Your platform's [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/): Xcode/Command Line Tools on macOS; Microsoft C++ build tools and WebView2 on Windows; WebKitGTK and the documented native libraries on Linux.
- Internet access for the initial dependency installation. The resulting application does not need internet access to start.

From the repository root:

```sh
npm ci
npm run desktop
```

Build the desktop application and platform bundles:

```sh
npm run desktop:build
```

These scripts build the CPU AI worker, stage its target-specific sidecar, and invoke Tauri. The first build is substantial because it compiles the inference runtime. They do **not** download model weights. `npm run dev` alone is a browser preview: local data access and settings persistence require the desktop application.

To experiment with a compatible GGUF you already have, see the [worker instructions](crates/ai-worker/README.md#actual-local-inference-test). This is an explicit developer test, not a model installer.

## Data and privacy

Settings → 데이터 및 개인정보 displays the actual database location and storage size. Rhythm uses Tauri's platform-specific application data directory and a `rhythm.sqlite3` database. Closing the main window exits the application and requests worker shutdown; there is no tray/background mode. Backup and restore are not implemented yet.

## Platforms and verification

macOS is the primary development platform. Windows and Linux build checks are configured in GitHub Actions; configuration alone is **not** evidence that they have passed or that runtime behavior has been tested.

On macOS Apple Silicon, the native debug `.app` was built and launched with its bundled worker. Desktop theme, extension, and energy-profile changes were exercised and checked in SQLite across app restarts; normal quit and window-close cleaned up the worker. An unexpected worker termination updated AI status while leaving the app running. Frontend checks, Rust tests, and Clippy passed. A real CPU inference test with an existing **Qwen3-0.6B-Q8_0.gguf** produced text and shut down cleanly.

Metal compilation was blocked by the missing Apple Metal Toolchain, despite Xcode being installed; CPU remains the default. Qwen3.5 artifacts, CUDA/GPU execution, release-optimized installers, and Windows/Linux runtime behavior remain unverified. See [the roadmap](docs/ROADMAP.md#v010--foundation) for remaining verification gates.

## Technology

Tauri 2 · Rust · Tokio · Svelte 5 · TypeScript · Tailwind CSS · SQLite with FTS5 available for future search · mistral.rs in a separate process · Pretendard · npm

[Roadmap](docs/ROADMAP.md) · [Architecture](docs/ARCHITECTURE.md) · [Design](docs/DESIGN.md)

## License and assets

Rhythm is [MIT licensed](LICENSE), © Jihoon Lee. The original Beat logo is included under the same license. [Pretendard](https://github.com/orioncactus/pretendard) is distributed under the SIL Open Font License 1.1; its [license](assets/licenses/Pretendard-OFL.txt) is retained in this repository and packaged as an application resource. Dependency licenses remain their respective authors' licenses.
