# Cybercampus connector

Area instructions for `src-tauri/src/archive/cybercampus/`, `CybercampusSettings.svelte`, `CybercampusTasks.svelte` and `storage/cybercampus.rs`. The root `AGENTS.md` (project-wide rules) applies too.

- `src-tauri/src/archive/cybercampus/`: the opt-in eClass connector (#86): `client.rs` session transport, `credentials.rs` OS credential store, `parser.rs`, sync in `mod.rs`, synthetic `fixtures/`. Courses/deadlines persist in `storage/cybercampus.rs`.

- **Cybercampus connector (user decisions, #86):** opt-in source `cybercampus`, owned by its own `cybercampus` extension, reads only the logged-in user's own current-term courses; `Controller::execute` dispatches its family to `cybercampus::sync`, so subscription, scheduler, cancellation and status are the archive's. Its flow is ported from rycont/eclass-cli (MIT, permission from its author). Course notices are documents (`<course key>-<seq>`, category = course name) with session-downloaded attachments/editor images; assignments and TODO deadlines live in `cybercampus_tasks`, replaced per course on success. Invariants: the login (학번 + SAINT password) lives only in the OS credential store and is sent only to `/ilos/lo/login.acl`; cookies stay in memory; requests go only to the fixed read-only routes in `client.rs::ROUTES` (never a submit/post/delete/confirm route); a rejected login stops all automatic retries until the user saves it again; fetching a notice counts as an eClass view, so unchanged list rows (title/preview fingerprint in `documents.modified_at`) are never refetched; the first sync is historical. Its documents are personal data: AI work on them stays local (classification and search treat them like other documents). Ordinary tests use the scripted fake in `cybercampus/tests.rs`; a live check needs the user's account and explicit approval, runs only against a temporary profile, and must not commit captured pages or credentials. Fixtures are synthetic (`cybercampus/fixtures/README.md`).

## Focused checks

```sh
cargo test --locked -p rhythm archive::cybercampus
npx playwright test tests/ui/cybercampus.spec.ts
```
