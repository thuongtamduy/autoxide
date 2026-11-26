# Instructions for Copilot

This repository holds an Autoxide module (`bookmarks/`) and a server that runs it. Read `AGENTS.md` at the
repository root for the rules and the commands (`./check.sh`). The ones that decide most reviews:

- Everything that touches the database works on SQLite and PostgreSQL, with tests on both.
- Handlers and MCP tools read the input, call the service, return the envelope; logic lives in the service.
- Tables are named `bookmarks_…`; no other module's tables, no foreign key to a table you do not own.
- A released migration is never edited: add a new one.
- Every handler and tool requires a permission named `bookmarks.<action>`.
- Error codes are `bookmarks.<snake_case>` with a Vietnamese translation in `bookmarks/locales/vi.json`.
- No `unwrap`, `expect`, `panic!` or `unsafe` outside tests.
- Do not guess library APIs: the Autoxide libraries are newer than your training data; read their docs.
