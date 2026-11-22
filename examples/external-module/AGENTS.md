# AGENTS.md

Rules for AI agents (and people) working in this repository. `CLAUDE.md` links to this file.

This repository holds an [Autoxide](https://github.com/thuongtamduy/autoxide) module (`bookmarks/`) and a
server that runs it next to the official modules (`server/`). The guide that explains every part:
[Writing a module in your own repository](https://github.com/thuongtamduy/autoxide/blob/main/docs/guides/writing-a-module.md).

## Rules

1. **Both databases.** Everything that touches the database — migrations, entities, queries, tests —
   runs on SQLite and PostgreSQL. Use the SeaORM schema builder, never SQL that only one of them
   understands. A feature that works on one database only is not done.
2. **Only public contracts.** Use `autoxide_kernel` and the `api` module of other modules
   (`autoxide_<name>::api::…`); never their internals. Talk to another module through the trait it
   provides (`context.require::<dyn Trait>()`), never through its tables. No foreign key to a table you
   do not own: keep the id only.
3. **Your tables only.** Every table is named `bookmarks_…`, has a UUIDv7 id made in the application,
   and — when it holds a customer's data — a `tenant_id` column that starts every key and index. A
   released migration is never edited: add a new one. Event and job tables come from
   `autoxide_kernel::migration::helper_tables`.
4. **Logic in the service.** Handlers and MCP tools read the input, call the service, return the
   envelope. Every route is registered with its OpenAPI description; every handler and tool requires a
   permission named `bookmarks.<action>`.
5. **Errors and text.** Error codes are `bookmarks.<snake_case>`, with a Vietnamese translation in
   `bookmarks/locales/vi.json`. No `unwrap`, `expect` or `panic!` outside tests; no `unsafe`.
6. **Tests.** New code has tests that use `TestDatabase::run` and `TestApp::builder` from
   `autoxide-testkit` — never a fixed database. Writes that can race have a concurrent test.

## Commands

```bash
./check.sh                                    # format, lints, every test on SQLite
POSTGRES_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres ./check.sh   # … and on PostgreSQL
cargo run -p acme-server -- migrate up        # create the tables
cargo run -p acme-server -- serve             # run the server
```

The architecture rules of the core are checked from a checkout of Autoxide:
`cargo xtask check-module <path to this repository>/bookmarks`. A change is done when `./check.sh` is
green on both databases and `check-module` reports no violations.
