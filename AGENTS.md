# AGENTS.md

Rules for AI coding agents (Codex, Cursor, Copilot, Claude Code, …) and for people working with them in this
repository. Short on purpose: the full rules for the project's own agents are in [`CLAUDE.md`](CLAUDE.md)
(Vietnamese, internal). If something here and there disagree, the stricter one wins; ask the maintainers.

## What this is

Autoxide is a Rust backend core: REST + OpenAPI, an MCP endpoint for AI agents, business modules plugged in
on top. It runs on **SQLite and PostgreSQL** (both, always). The core stays neutral: no business concept of
one industry (orders, stock, patients) goes into it; that belongs in a module.

## First steps

```bash
cargo xtask doctor                      # is this machine ready? (read-only)
cargo run -p autoxide -- demo           # a running server with an admin password on screen
./gate.sh --quick                       # the definition of "done" before every commit
```

- What to read: [`docs/GETTING-STARTED.md`](docs/GETTING-STARTED.md), [`STACK.md`](STACK.md) (what is used and why),
  [`docs/MODULE_GUIDE.md`](docs/MODULE_GUIDE.md) (writing a module), [`CONTRIBUTING.md`](CONTRIBUTING.md).
- Do not guess a library's API from memory: the libraries here are newer than most models know. Read the
  pinned version's docs, then let `cargo check` confirm.

## Rules that decide most reviews

1. **Both databases.** Anything that touches the database (migration, query, test) must work on SQLite and
   PostgreSQL. Never skip a test for one of them. Run database tests on PostgreSQL too (`TEST_DATABASE_URL`).
2. **Layers.** A handler only reads input, calls a service and returns an envelope; logic lives in the service
   layer. REST and MCP tools call the same service.
3. **Modules own their tables.** No reading, writing or joining another module's tables, no foreign key across
   modules (keep the id only). Call another module through the trait it publishes in `api`.
4. **Permissions, not role names.** `principal.require(&NOTES_WRITE)?`. Tables of a tenant carry `tenant_id` and
   every query goes through `.scoped(&tenant)`.
5. **Data.** UUIDv7 ids made in the application; money as integer minor units plus a currency code; times in UTC;
   a migration that is committed is never edited (add a new one, with `down`).
6. **No `unsafe`, no `unwrap()` / `expect()` / `panic!`** outside tests: return an error. One error type
   (`autoxide_kernel::error::Error`); the error `code` is stable and is also the i18n key.
7. **Responses** have three shapes only: `Data<T>`, `Page<T>` (every list, keyset pagination) and `Problem`
   (RFC 9457). Every route is documented in OpenAPI (a `///` summary and a description for each response).
8. **Config** goes through `Setting` and `ConfigReader`, never `std::env::var`. Never commit secrets; never log
   secrets, tokens, passwords or request bodies.
9. **Tests** for every change, in `tests/integration/` with a `mod` line in `main.rs` (no stray `tests/<name>.rs`).
   Database tests get their database from `TestDatabase::run`, apps from `TestApp::builder`.
10. **A red gate is fixed in the code**, never by loosening a lint or skipping a test. When `cargo xtask check`
    reports a violation it prints a `how to fix:` line.

## Before you push

- `./gate.sh --quick` is green, and `cargo xtask regen --check` after changing the API or settings
  (`cargo xtask regen` rewrites the generated files: `docs/openapi.json`, `.env.example`, the SDK).
- A change users can see gets a fragment `changelog.d/<name>.<section>.md`; do not edit `CHANGELOG.md`.
- Docs for outsiders come in English and Vietnamese (`X.md` + `X.vi.md`), with the same content.
- A long-lived choice (a library, a convention, an exception to a rule) is recorded as a file
  `docs/decisions/D-<YYYYMMDD>-<name>.md`, then `cargo xtask decisions`.

## Writing your own module

Start from [`docs/MODULE_GUIDE.md`](docs/MODULE_GUIDE.md) and `cargo xtask new-module`. A module outside this
repository: [`docs/guides/writing-a-module.md`](docs/guides/writing-a-module.md); `cargo xtask check-module <dir>`
checks it with the same rules as above.
