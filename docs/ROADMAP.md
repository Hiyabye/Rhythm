# Roadmap

Version numbers describe product milestones, not promises that future functionality exists. Rhythm stays local-first and Korean-first throughout.

## v0.1.0 — Foundation

**Status: implementation present; release verification remains open.**

Implemented in the foundation:

- Tauri 2 application configuration, Rust core, Svelte 5/TypeScript frontend, npm lockfile, and 0.1.0 metadata.
- Five main navigation sections and intentional, honest empty states.
- Pretendard typography, initial Beat branding/icons, responsive light/dark/system themes.
- SQLite startup initialization and a transactional first migration.
- Persistent theme, energy profile, interests, and extension preferences.
- Built-in extension registry/lifecycle and unavailable `sogang_notices` definition.
- Separate mistral.rs 0.8.1 CPU worker with versioned stdin/stdout protocol, lifecycle operations, local inference request, and no automatic model downloads.
- Main-process worker ownership, failure reporting, deadlines, and shutdown handling.
- Cross-platform CI configuration and developer documentation.

Verified on macOS Apple Silicon:

- [x] Frontend type check and production build.
- [x] Native debug `.app` build, launch, and bundled worker placement.
- [x] Desktop theme/extension/energy changes persisted through SQLite and app restarts.
- [x] Normal quit and native window-close cleaned up the worker; a worker termination updated the UI without terminating the app.
- [x] Core migration/persistence, validation, extension, and worker failure-isolation tests with the real-worker environment variable set.
- [x] Real CPU generation using an existing Qwen3-0.6B-Q8_0.gguf and clean loaded-model shutdown.

Remaining verification gates:

- [ ] Complete keyboard/screen-reader and responsive-layout testing.
- [ ] Observe Windows and Linux CI build results; runtime behavior remains a separate test.
- [ ] Verify release-optimized installers and their platform-specific packaging.
- [ ] Validate Qwen3.5 2B/4B/9B artifacts and hardware backends before claiming support.

Metal compilation was attempted but blocked by the missing Apple Metal Toolchain. Qwen3.5 2B/4B/9B remain planned model choices, not verified support. Successful Qwen3 CPU generation is not evidence of GPU support. No weights were downloaded by this implementation; ordinary startup remains independent of models.

## v0.2.0 — Local university notice archive

Planned: real Sogang notice ingestion, original and normalized content, revisions, incremental synchronization, local attachments as needed, saved/read states, and Korean search with SQLite FTS5. Previously downloaded material must remain available offline. No ingestion is implemented in v0.1.0.

## v0.3.0 — Local AI model management

Planned: compatible model selection and downloads, first-run setup, explicit storage/resource information, local summaries and classifications, and lazy processing. Validate exact Qwen3.5 artifacts and hardware backends before claiming support. Keep weights separate from application installers.

## v0.4.0 — Personalization

Planned: user-controlled persistent memory, explicit preferences and feedback, explainable relevance ranking, and personalized recommendations. Existing interest text is stored only; automatic learning/ranking is not implemented.

## v0.5.0 — Proactive operation

Planned: background execution, native notifications, scheduler, and energy-aware processing. Apply energy profiles to frequency, residency, and background work. Disabling extensions must cancel their work and notifications without deleting data.

## v0.9.0 — Integration and distribution readiness

Planned: integration testing, usability refinement, installer/updater work, resource/error recovery validation, and distribution readiness. GitHub Releases and Tauri distribution are the intended direction; publishing/updating is not configured as an operational service today.

## v1.0.0 — Stable release

Planned: stable local-first daily command-center experience with validated platform behavior, reliable archives and AI integration, clear privacy controls, and supported distribution.

## After v1.0.0

Possible built-in extensions:

- **Cybercampus:** announcements, assignments, course information, and deadlines; port appropriate functionality from `rycont/eclass-cli` into a native Rust connector.
- **Opportunities:** competitions, extracurricular activities, internships, research opportunities, and jobs.
- **Calendar and Tasks:** schedules, deadlines, reminders, and daily planning.
- **Email:** defer until OAuth and provider restrictions can be handled deliberately.
- **Personal Agent:** context-aware proposals and workflows whose consequential actions remain under user control.

These extensions should share the archive, search, and personalization core. No marketplace, cloud synchronization, or autonomous external-action infrastructure is part of the foundation.
