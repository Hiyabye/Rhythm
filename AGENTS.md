# Rhythm development

## Scope and architecture

Rhythm is a Korean-first, local-first Tauri 2 desktop command center. Read `docs/ARCHITECTURE.md` before changing component boundaries and `docs/ROADMAP.md` before adding product scope. The current release is 0.1.0 foundation: no notice ingestion, real archive search, model installer, background scheduler, notifications, or cloud services.

- `src/`: Svelte 5 runes, TypeScript, custom components and Tailwind CSS. `App.svelte` owns navigation/settings; `lib/types.ts` mirrors command payloads.
- `src-tauri/src/main.rs`: Tauri commands and application lifecycle/state.
- `src-tauri/src/storage/`: rusqlite store; migrations are in `src-tauri/migrations/`.
- `src-tauri/src/extensions/`: built-in extension metadata, registry, and lifecycle.
- `src-tauri/src/intelligence/`: Tokio process ownership and worker protocol client.
- `crates/ai-worker/`: isolated mistral.rs runtime and JSON-lines executable. Read its `README.md` before worker/inference changes.
- `scripts/desktop.mjs`: host worker build/staging and Tauri invocation.
- `assets/branding/beat.svg`: vector icon source; desktop icons are in `src-tauri/icons/`.

There are only two Cargo workspace members. Do not add a crate or generic extension/plugin infrastructure without a concrete need.

## Setup and commands

Use a current stable Rust toolchain (at least 1.90), Node.js 22.12+ and npm, and the OS-specific [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). Keep `cargo` and `rustc` on PATH. Use the committed root `Cargo.lock` and `package-lock.json`.

```sh
npm ci
npm run desktop                 # builds/stages CPU worker, then native dev app
npm run desktop:build           # release worker + native application/bundles
npm run desktop:build -- --no-bundle  # native build without installer generation
npm run check                   # Svelte/TypeScript diagnostics
npm run build                   # frontend production build only
cargo fmt --all
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

Tauri checks need the worker staged at `src-tauri/binaries/rhythm-ai-worker-<host-triple>[.exe]`. The desktop scripts create it; CI explicitly stages a debug worker before Clippy/tests. Running Tauri directly on a fresh checkout bypasses this step. Generated sidecars are ignored; icons are committed. The helper currently targets the host platform and assumes Cargo's default workspace `target/` directory.

For core process lifecycle/crash-isolation tests, build the worker and set `RHYTHM_WORKER_TEST_BIN` to its absolute executable path before running `cargo test -p rhythm`. Without that variable, the current core real-worker test returns early: a passing test result does not establish process integration. Worker crate process tests launch their own test-built binary. The actual-model test is separately ignored and opt-in via `RHYTHM_TEST_MODEL`; never call it verified without real generated output.

Default worker builds include real mistral.rs 0.8.1 CPU inference. Metal/CUDA are optional worker features; the initial Metal attempt was blocked by missing Apple Metal Toolchain. Do not install toolchains or download weights implicitly. See `crates/ai-worker/README.md` for reproducible local inference and known limitations.

## Implementation constraints

- Keep user-facing UI, feedback, and errors in natural Korean. Internal identifiers/docs may use English.
- Persist settings in Rust/SQLite, not frontend localStorage. Browser previews must clearly state desktop-only functionality instead of simulating persistence.
- SQLite lives at Tauri's `app_data_dir()/rhythm.sqlite3`; do not hardcode platform-specific home paths.
- New schema changes require ordered transactional migrations and upgrade tests. Never rewrite released migrations; reject unknown newer schema versions. Only create tables needed by implemented behavior.
- Extension availability and persisted opt-in are separate. `sogang_notices` is unavailable in 0.1.0. Disabling must stop jobs/notifications and hide extension UI without deleting data; future lifecycle implementations must uphold the existing `stop` contract.
- No localhost AI HTTP, cloud inference/storage, telemetry, or separately installed inference services. Model ownership remains in the worker; keep stdout exclusively for versioned protocol messages and diagnostics on stderr.
- Preserve CPU startup without weights or GPU tooling. Label planned model/backend support as unverified until tested against specific artifacts/hardware.
- Keep `src/lib/types.ts` and Rust command payloads synchronized. Validate persisted/input values at the backend boundary.
- Use Rustfmt; frontend code should pass `npm run check` without warnings. There is no separate configured frontend formatter/linter.

## Verification and documentation

Run affected tests and checks, inspect the final diff, and report exact evidence and limitations. At minimum, settings changes need persistence/reopen tests; worker changes need process protocol/lifecycle tests. Native build success does not prove rendering, keyboard accessibility, restart persistence, clean loaded-model shutdown, or Windows/Linux runtime behavior.

Update `docs/ARCHITECTURE.md`, `docs/DESIGN.md`, and `docs/ROADMAP.md` when implementation changes their claims. Keep README focused on users and basic run/build instructions; put development workflows here. Update release verification gates only after the corresponding checks actually ran. Do not introduce community-management templates, changelogs, publishing, or updater infrastructure outside the roadmap's scope.
