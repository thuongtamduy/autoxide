# Writing a module in your own repository

Vietnamese: [`writing-a-module.vi.md`](writing-a-module.vi.md).

Autoxide is a core that modules plug into. Your module does not have to live in the Autoxide repository, and you do not fork anything: you keep a repository of your own with **your module** (a library crate) and **your server** (a five-line binary that runs the official modules plus yours). Your module gets everything a built-in module gets: settings, migrations, permissions, events, background jobs, REST routes in the OpenAPI document, MCP tools for AI agents, and translations.

This guide goes from an empty directory to a module that runs, has tests on SQLite and PostgreSQL, and is part of a server. Everything shown here is the example in [`examples/external-module`](../../examples/external-module), which CI builds and tests on every change — when in doubt, read the file there. The reference for each part of the module contract is [`docs/MODULE_GUIDE.md`](../MODULE_GUIDE.md).

## What you need

- Rust, in the version of the Autoxide release you build on (its `rust-toolchain.toml`) or newer; `rustup` installs it.
- Nothing else for SQLite. For the PostgreSQL tests, a PostgreSQL 16 or newer server and a user that may `CREATE DATABASE` (for example `docker run --rm -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:16-alpine`).

## 1. The workspace

One repository, two crates: the module and the server. To get this whole layout generated — a copy of the example with your names — run `cargo xtask new-module --external <dir> <module> <entity>` in a checkout of Autoxide (for example `cargo xtask new-module --external ../acme bookmarks bookmark`), then read on to see what each part does.

```text
acme/
├── Cargo.toml          the workspace
├── rust-toolchain.toml
├── clippy.toml
├── bookmarks/          your module: crate `acme-bookmarks`
│   ├── Cargo.toml
│   ├── locales/vi.json
│   └── src/
└── server/             your server: crate `acme-server`, binary `acme`
    ├── Cargo.toml
    └── src/main.rs
```

The workspace `Cargo.toml` names the core once. Depend on a **release tag**, so that your build does not change until you choose a newer release:

```toml
[workspace]
resolver = "3"
members = ["bookmarks", "server"]

[workspace.dependencies]
autoxide = { git = "https://github.com/thuongtamduy/autoxide", tag = "vX.Y.Z" }
autoxide-kernel = { git = "https://github.com/thuongtamduy/autoxide", tag = "vX.Y.Z" }
autoxide-testkit = { git = "https://github.com/thuongtamduy/autoxide", tag = "vX.Y.Z" }

async-trait = "0.1"
axum = "0.8"
sea-orm = { version = "2.0", default-features = false, features = ["macros"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
tracing = "0.1"
utoipa = "6.0"
utoipa-axum = "0.3"
uuid = { version = "1", features = ["v7"] }
validator = { version = "0.21", features = ["derive"] }
```

Three things to know:

- **All three core crates use the same tag.** Cargo then builds one copy of the kernel; two tags would give two kernels whose types do not match.
- **The libraries below the core are part of the module contract** (`axum`, `sea-orm`, `utoipa`…): keep the major versions of the release you build on — the table "Stability of the module contract" in [`MODULE_GUIDE.md`](../MODULE_GUIDE.md) lists them. The core turns on the features it needs (database drivers, the async runtime); you add only what your code uses. For `sea-orm`, turn the default features off as above.
- **Library versions.** A git dependency does not bring the core's `Cargo.lock`: cargo picks the newest compatible version of every library. To start from exactly the versions the release was tested with, copy the `Cargo.lock` of the tag into your repository before the first build (cargo trims what you do not use). Commit your `Cargo.lock`.

[The example's `Cargo.toml`](../../examples/external-module/Cargo.toml) also sets the lints of the core (no `unsafe`, no `unwrap` outside tests) and a build profile that keeps test programs small; copy both, with [`clippy.toml`](../../examples/external-module/clippy.toml).

## 2. The module crate

`bookmarks/Cargo.toml` lists what the module's code uses; the test kit is a dev-dependency:

```toml
[package]
name = "acme-bookmarks"
version = "0.1.0"
edition = "2024"

[dependencies]
async-trait.workspace = true
autoxide-kernel.workspace = true
axum.workspace = true
sea-orm.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
utoipa.workspace = true
utoipa-axum.workspace = true
uuid.workspace = true
validator.workspace = true

[dev-dependencies]
autoxide-testkit.workspace = true
tokio.workspace = true
```

The module's **name** — here `bookmarks` — is the prefix of everything it owns: tables `bookmarks_…`, routes under `/api/v1/bookmarks`, permissions, events, jobs, error codes and MCP tools named `bookmarks.…`. Choose a name no official module has.

A module crate shows exactly two things to other crates: `pub mod api` (traits and types other modules may use) and `pub fn module()`. Everything else is private. Start with one file per role:

| File | What it holds |
| --- | --- |
| [`src/lib.rs`](../../examples/external-module/bookmarks/src/lib.rs) | the `Module` implementation: what the module declares to the core |
| [`src/api.rs`](../../examples/external-module/bookmarks/src/api.rs) | the contract for other modules |
| [`src/config.rs`](../../examples/external-module/bookmarks/src/config.rs) | settings |
| [`src/migration/`](../../examples/external-module/bookmarks/src/migration/mod.rs) | tables |
| [`src/entity.rs`](../../examples/external-module/bookmarks/src/entity.rs) | the SeaORM entity of each table |
| [`src/permissions.rs`](../../examples/external-module/bookmarks/src/permissions.rs) | permissions |
| [`src/events.rs`](../../examples/external-module/bookmarks/src/events.rs) | events |
| [`src/service.rs`](../../examples/external-module/bookmarks/src/service.rs) | all the logic |
| [`src/http.rs`](../../examples/external-module/bookmarks/src/http.rs) | REST handlers |
| [`src/tools.rs`](../../examples/external-module/bookmarks/src/tools.rs) | MCP tools |
| [`src/retention.rs`](../../examples/external-module/bookmarks/src/retention.rs) | a scheduled background job |
| [`locales/vi.json`](../../examples/external-module/bookmarks/locales/vi.json) | Vietnamese texts |
| [`src/tests.rs`](../../examples/external-module/bookmarks/src/tests.rs) | tests |

`lib.rs` is the table of contents. Every function of `Module` except `name` has a default, so a module declares only what it has:

```rust
pub mod api;
// … private modules …

pub fn module() -> impl Module {
    BookmarksModule::default()
}

impl Module for BookmarksModule {
    fn name(&self) -> &'static str {
        "bookmarks"
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(BookmarksConfig::read(reader)?);
        Ok(())
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self.config.clone().unwrap_or_default();
        let service = Arc::new(BookmarkService::new(context.outbox()?, config));
        self.service = Some(service.clone());
        context.provide::<dyn api::Bookmarks>(service)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    // … events, translations, subscriptions, jobs, schedules, tools …
}
```

The rules that matter most, each shown in the example file named:

- **Tables** (`migration/`, `entity.rs`): named `bookmarks_…`; ids are UUIDv7 made in the application; a table that holds a customer's data has a `tenant_id` column, and every key and index starts with it; no foreign key to a table of another module — keep the id only. Migrations are written with the schema builder, not SQL, so one migration runs on both databases, and each has a `down`. A released migration is never edited: add a new one.
- **Logic in the service** (`service.rs`): every method takes the `TenantContext` of the customer, reads through `tenant.database().read()`, writes in one short `begin_write()` transaction, and filters every query with `.scoped(tenant)`. A write records its event in the same transaction (`Outbox::record`).
- **Handlers and tools only call the service** (`http.rs`, `tools.rs`): take the input (`ValidJson<T>`), check the permission (`principal.require(&BOOKMARKS_WRITE)?`), call one service method, return `Data<T>`, `Page<T>` or `Created<T>`. Routes are registered with `routes!`, which puts them in the OpenAPI document; the `///` comment and every response `description` are copied there, so write them in English for the API's callers.
- **Errors** carry a stable `code` with the module prefix (`bookmarks.url_taken`) and an English text; the Vietnamese text goes under the same code in `locales/vi.json`.
- **Settings** (`config.rs`) are declared once with `Setting` and read through `ConfigReader`, never `std::env::var`. Use a prefix of your own (`ACME_…`); `AUTOXIDE_…` belongs to the core.
- **Both databases, always.** Do not use a type, function or syntax only one of them has.

## 3. The server

`server/Cargo.toml`:

```toml
[package]
name = "acme-server"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "acme"
path = "src/main.rs"

[dependencies]
acme-bookmarks = { path = "../bookmarks" }
autoxide.workspace = true
```

`server/src/main.rs` is the whole program:

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    autoxide::Autoxide::new()
        .name("acme")
        .version(env!("CARGO_PKG_VERSION"))
        .module(acme_bookmarks::module)
        .run()
}
```

`Autoxide::new()` is the official program with every official module; `.module(…)` adds yours, one line each. Your modules start after the official ones, in the order added, so they can `require` any trait an official module provides, and a module of yours that uses another of yours comes after it. `.run()` gives the binary the standard command line.

## 4. Run it

From the workspace directory. The first build compiles the whole core and takes a while.

```bash
cargo run -p acme-server -- config                # every setting, yours included; checks them
cargo run -p acme-server -- migrate up            # creates the tables of every module, yours included
AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL=admin@example.com \
AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD='correct horse battery staple' \
  cargo run -p acme-server -- serve               # http://127.0.0.1:8080; creates the first administrator
```

Which modules run is a setting, `AUTOXIDE_MODULES` (see [`docs/MODULES.md`](../MODULES.md)): by default every official module and every module you added; `AUTOXIDE_MODULES=official,-bookmarks` turns yours off, and a module turned on brings the modules it names in `Module::depends_on`. `acme modules` lists every module with its tier, whether it is on, and what it depends on (`--json` for programs). `acme openapi` and `acme mcp-manifest` always print the full contract.

With no settings the server keeps its data in SQLite under `./data`. For PostgreSQL set `DATABASE_URL=postgres://…`; nothing else changes. `acme config --env-example > .env.example` writes a documented list of every setting, and `acme doctor` checks an installation step by step.

In another terminal, sign in and call your module. The administrator created on first start holds every permission, including the ones your module declares:

```bash
export API=http://127.0.0.1:8080
TOKEN=$(curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}' \
  | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')
curl -sS "$API/api/v1/bookmarks" -H "Authorization: Bearer $TOKEN" \
  --json '{"url":"https://www.rust-lang.org/","title":"Rust"}'
curl -sS "$API/api/v1/bookmarks" -H "Authorization: Bearer $TOKEN"
```

Your routes are in the API document at `$API/api/v1/openapi.json` (also `acme openapi`), your tools in `acme mcp-manifest` and at the MCP endpoint, and everything the module declares is listed at `GET /api/v1/kernel/modules`.

## 5. Tests on both databases

Tests get an empty database each from `TestDatabase::run`; `TestApp` builds your module the way the server does and calls it over REST and with a real MCP client:

```rust
#[tokio::test]
async fn a_bookmark_can_be_created() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("ada", 0xada, &["bookmarks.read", "bookmarks.write"])
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let answer = app
            .post("/api/v1/bookmarks")
            .token("ada")
            .json(json!({"url": "https://www.rust-lang.org/", "title": "Rust"}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::CREATED);
    })
    .await
    .unwrap();
}
```

The database is chosen by `TEST_DATABASE_URL`: unset is SQLite, a `postgres://` URL is PostgreSQL. Run every test on both:

```bash
cargo test --workspace
TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres \
  cargo test --workspace -- --test-threads=4
```

[`src/tests.rs`](../../examples/external-module/bookmarks/src/tests.rs) of the example shows the tests a module should have: REST and MCP give the same result, permissions are checked, a concurrent write (two requests keeping the same link) lets one through, two tenants never see each other (`leak_check`), the tables follow the rules (`schema_ownership_violations`, `schema_tenancy_violations`), the job and the settings work. [`server/tests/cli.rs`](../../examples/external-module/server/tests/cli.rs) runs the real binary: `migrate up`, `openapi` and `mcp-manifest` include the module. [`check.sh`](../../examples/external-module/check.sh) runs format, lints and both test runs; keep a script like it and run it in CI.

## 6. The rules of the core, checked on your crate

Inside the Autoxide repository, `cargo xtask check` enforces the module rules. For a crate anywhere else, run the same source rules from a checkout of Autoxide at the tag you build on:

```bash
cargo run --manifest-path /path/to/autoxide/xtask/Cargo.toml -- check-module /path/to/acme/bookmarks
```

It checks: the crate exposes only `pub mod api` and `pub fn module`; `api` does not reach into the crate's internals; routes go through `routes!`; handlers and tools do not touch the database; safety lints are not switched off in `src/`; settings are not read with `std::env::var`; hand-written SQL is built with `autoxide_kernel::database::statement`; every error code and description has a Vietnamese text; entities have `tenant_id`; queries are filtered by tenant; MCP tools pass the caller to the service where REST limits by scope.

What it cannot see lives in the database and is checked by your tests (step 5): the table prefix, no foreign key to another module, `tenant_id` first in every key and index, no data crossing tenants.

## 7. Upgrading to a newer Autoxide

1. Read `CHANGELOG.md` of the new release: breaking changes for modules are under **Changed** and **Removed**, the Rust items under `### Breaking (Rust)`.
2. Change the tag in your workspace `Cargo.toml` (all three core crates) and build: cargo updates `Cargo.lock` for the core crates. Run your tests on both databases.
3. On each server: `acme migrate up`, then start the new binary — as for the official server ([`docs/DEPLOY.md`](../DEPLOY.md)).

What the core promises: while its version is `0.x`, a breaking change to the module contract comes only in a minor release (`0.17` → `0.18`), never in a patch release; an item that is going away is `#[deprecated]` for at least one minor release first; the `Module` trait only grows by functions with a default body. From `1.0` the contract follows Semantic Versioning. The full statement is in [`MODULE_GUIDE.md`](../MODULE_GUIDE.md), "Stability of the module contract".

The core crates are not on crates.io yet; until they are, the git tag is the way to depend on them.

## Checklist

- [ ] The module crate exposes only `pub mod api` and `pub fn module`
- [ ] Tables named `<module>_…`, UUIDv7 ids, `tenant_id` first in every key and index, migrations with `down`
- [ ] Permissions declared; every handler and tool requires one
- [ ] Logic only in the service; each write records its event in the same transaction
- [ ] Every route documented: summary, permission, every response
- [ ] MCP tools for what agents should be able to do
- [ ] Error codes prefixed with the module name and translated
- [ ] Data about a user deleted on `USER_DELETED`
- [ ] Settings with a prefix of your own, declared with `Setting`
- [ ] Tests on SQLite and PostgreSQL, including concurrent writes, tenant isolation and the table rules
- [ ] `cargo xtask check-module` reports no violations
