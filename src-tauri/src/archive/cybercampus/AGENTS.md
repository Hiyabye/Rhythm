# Cybercampus connector

Area instructions for `src-tauri/src/archive/cybercampus/`, `CybercampusSettings.svelte`, `CybercampusTasks.svelte` and `storage/cybercampus.rs`. The root `AGENTS.md` (project-wide rules) applies too.

- `src-tauri/src/archive/cybercampus/`: the opt-in eClass connector (#86, user decisions): `client.rs` session transport, `credentials.rs` OS credential store, `parser.rs`, sync in `mod.rs`, synthetic `fixtures/`. Courses/deadlines persist in `storage/cybercampus.rs`.
- **Contract:** `docs/ARCHITECTURE.md` "Cybercampus connector" (data model, sync, session and credentials, read-only boundary, privacy). Read it before changing the connector and change it with the code.

## Rules

- Requests go only to the fixed read-only routes in `client.rs::ROUTES`; never add a submit, post, delete or confirm route.
- The login (학번 + SAINT password) lives only in the OS credential store and is sent only to `/ilos/lo/login.acl`; never put it in SQLite, backups, logs or diagnostics. Cookies stay in memory.
- A rejected login must stop every automatic retry until the user saves the login again (account-lock protection).
- Fetching a notice counts as an eClass view: never refetch unchanged list rows (fingerprint in `documents.modified_at`). The first sync is historical and announces nothing.
- Its documents are personal data: AI work on them stays local.
- Ordinary tests use the scripted fake in `cybercampus/tests.rs`. A live check needs the user's account and explicit approval, runs only against a temporary profile, and must not commit captured pages or credentials. Fixtures are synthetic (`cybercampus/fixtures/README.md`).

## Focused checks

```sh
cargo test --locked -p rhythm archive::cybercampus
npx playwright test tests/ui/cybercampus.spec.ts
```
