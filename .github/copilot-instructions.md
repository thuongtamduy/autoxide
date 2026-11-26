# Instructions for Copilot

This is Autoxide, a Rust backend core (REST + OpenAPI, MCP, business modules) that runs on SQLite and
PostgreSQL. Read `AGENTS.md` at the repository root for the rules and commands (`cargo xtask doctor`,
`./gate.sh --quick`). The ones that decide most reviews:

- Everything that touches the database must work on SQLite and PostgreSQL, with tests on both.
- Handlers only parse input, call a service and return an envelope; logic lives in the service layer.
- A module owns its tables: no other module's tables, no foreign key across modules.
- Check permissions (`principal.require(&PERMISSION)?`), not role names; tenant tables are queried through `.scoped(&tenant)`.
- No `unsafe`, no `unwrap()` / `expect()` / `panic!` outside tests; errors carry a stable `code`.
- Do not guess library APIs: the pinned versions are newer than your training data; read the docs.
- A red gate is fixed in the code, never by loosening a lint or skipping a test.
