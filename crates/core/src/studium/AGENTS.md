# Studium (reading-room seats)

Area instructions for `crates/core/src/studium/` (in `rhythm-core`), the app's `src-tauri/src/studium/` (commands, native mini window and tray), `src/lib/StudiumView.svelte`, `src/lib/StudiumMini.svelte`, `src/lib/StudiumSeatMap.svelte` and the `storage/studium*.rs` modules. The root `AGENTS.md` (project-wide rules) applies too.

- `crates/core/src/studium/`: separate fixed reading-room catalog (`rooms.rs`), pure UTF-8/CP949 seat parsers (`parser.rs`, optional geometry in `parser_layout.rs`), payloads (`model.rs`), bounded seat session over the crate transport (`network.rs`; the route validator is `rhythm_sources::studium`), shared manual/automatic in-memory job controller (`controller.rs`, timer in `controller/automatic.rs`), the admission helpers at the Store boundary (`jobs.rs`) and the seat-alert runtime (`notification.rs`, clicks through `crate::shell::Ui`). The app re-exports them at `crate::studium::*` and keeps the commands (`src-tauri/src/studium/commands.rs`) and the native mini/tray owner (`views.rs`, which implements `crate::app::StudiumViews` so maintenance pauses it and turning Studium off closes the mini window). `src/lib/StudiumView.svelte` owns its multi-room selection, automatic controls, favorite/alert editors and seat results. Its collection is separate from archive sources and AI; seat alerts share proactive permission, quiet hours, rate policy and native senders.
- **Contracts live in `docs/ARCHITECTURE.md`:** "Studium seat checks" (preferences, favorites, controller, automatic timer, result states, source-time age policy, disable/maintenance), "Studium seat map", "Studium seat alerts", "Studium mini window and tray", and under "Transport, resources and safe rendering" the `Access::Studium` routes and the seat-origin TLS paragraph. Read the matching section before changing that part, and change the doc with the code.

## Rules

- **No implicit requests:** school requests come only from an explicit `studium_start` or an automatic session explicitly started by `studium_automatic_start`. Enablement, navigation, preference/favorite/alert/interval saves, search, map interaction, mini open/close/view/on-top/show-main, startup, re-enable, restore and maintenance recovery start no seat check and never resume automatic mode. Never add another scheduler.
- **Memory-only observations:** persist only the settings the schema table lists (rooms, display room, interval, favorites, alert rules/latches, bounded attempt accounting, hashed tokens, mini geometry). Never persist seat results, counts, numbers, source times, coordinates, raw HTML or history.
- **Honest results:** valid zero is distinct from unknown; a count-only fallback never borrows old numbers; receive time never replaces the source timestamp; count-only/failed/unchecked results never imply a preferred seat is free or a search has zero matches; stale results are labeled as previous observations. No predictions or reservations.
- **Stopping:** disablement, maintenance and exit stop and join the automatic session, the owning job, display work and notification handoffs outside the Store lock. Every admitted check holds an `AdmissionLease` until it ends; an idle timer does not.
- **Seat TLS (user decision):** `Access::Studium` alone uses native TLS with the pinned R46 root, for the server's CBC-only TLS 1.2 suites that Rustls lacks. Never fall back to the OS store, anchor an intermediate, accept invalid certificates, add another root, use a proxy or downgrade the protocol. Every other client must keep Rustls explicitly so the exception stays seat-only. Compatibility is a separate claim on each platform.
- **Alerts:** request notification permission only from the existing explicit settings controls; a reserved but uncertain OS handoff is never replayed automatically.
- **Map:** never execute source HTML/CSS/scripts or add map requests; keep ○/×/★/underline states independent of color; avoid hundreds of focus targets.
- **Fixtures** must be synthetic; never commit private Studium source or captured school HTML.
- **Evidence:** fake senders, mocked IPC/clicks and native compilation do not verify native display, click, lifecycle, tray or macOS behavior. Scope native claims to the [recorded validation runs](../../../../docs/ROADMAP.md#verification-status); DOM-triggered native IPC does not establish physical input on recreated WebView2 windows, OS tray menus, monitor recovery or installed-app behavior.

## Focused checks

```sh
cargo test --locked -p rhythm studium
cargo test --locked -p rhythm-core studium
npx playwright test tests/ui/studium.spec.ts tests/ui/studium-mini.spec.ts tests/ui/studium-map.spec.ts
```

`src-tauri/src/studium/live_tests.rs` (ignored) sends real requests to the seat server; run it only with explicit approval and never commit its output.
