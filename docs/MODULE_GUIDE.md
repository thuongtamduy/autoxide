# Writing a module

[Tiếng Việt](MODULE_GUIDE.vi.md)

Business features live in modules, not in the core. A module is one crate under
`modules/<name>` that owns its tables, declares what it needs from the core through the
`Module` trait, and is registered in exactly one place. The sample module
[`modules/notes`](../modules/notes) uses every part of the contract;
`cargo xtask new-module` starts a new module from it.

## Rules that the gate enforces

- **One crate, two public items.** A module exposes only `pub mod api;` (types and
  traits other modules may use) and `pub fn module()` (for the binary). Everything else
  is private. Other modules write `autoxide_<name>::api::…` and nothing else.
- **Own tables only.** Every table of module `notes` is named `notes_…`, including the
  migration history `notes_migrations`. Never read, write or join another module's
  tables, and never add a foreign key to them: keep the id only.
- **Both databases.** Everything runs on SQLite and PostgreSQL. Do not use a type,
  function or syntax only one of them has. Tests run on both.
- **Logic in the service.** REST handlers and MCP tools take input, check the
  permission, call one service method and return its result. They never touch the
  database: `cargo xtask check` stops a handler or tool that opens a connection.
- **Errors have a stable `code`** with the module prefix (`notes.title_taken`) and a
  Vietnamese translation in `locales/vi.json`.

`./gate.sh` checks all of this; a module is done when the gate is green.

## Step by step

### 1. Create the crate

Generate it from the sample module:

```bash
cargo xtask new-module bookmarks bookmark   # <module: plural> <entity: one record>
```

This copies `modules/notes` to `modules/bookmarks` and renames every `notes` / `note` (in
every case: crate name, `Module::name`, tables, permissions, events, error codes, types,
file names). The result passes the gate as it is; then rewrite the tables, the service,
the descriptions and `locales/vi.json` for your feature. To register it:

1. add `autoxide-<name> = { path = "modules/<name>" }` under `[workspace.dependencies]`
   in the root `Cargo.toml`;
2. add `autoxide-<name>.workspace = true` to `crates/app/Cargo.toml`;
3. add one line to `all()` in `crates/app/src/modules.rs`:
   `Box::new(autoxide_<name>::module())`. A module that uses another module's trait
   is registered after it.

The crate's `Cargo.toml` lists dependencies as `<library>.workspace = true`, with
`license.workspace = true` and `[lints] workspace = true`.

**File layout.** Start with one file per role, as in `notes`: `lib.rs` (the `Module`
implementation), `api.rs` (the contract for other modules), `config.rs`, `permissions.rs`,
`events.rs`, `entity.rs`, `migration/`, `service.rs`, `http.rs`, `tools.rs` and `tests.rs`.
When the module passes about 5,000 lines, group the service code by feature instead, one
folder per feature (`session/`, `user/` in `auth`; `entries/`, `publishing/` in `content`),
and keep `lib.rs`, `api.rs`, `config.rs`, `permissions.rs`, `events.rs`, `entity/`,
`migration/`, `http/` and `tools.rs` at the top, so every module reads the same way. Tests
longer than about 1,000 lines go in a `tests/` folder, one file per feature.

### 2. Tables: migrations and entities

Each migration is a file in `src/migration/` and a line at the end of `all()`. A
committed migration is never edited — add a new one. Every migration has a `down`.

- Ids are UUIDv7 made by the application (`Uuid::now_v7()`), never auto-increment.
- Times are UTC; money is an integer of the smallest unit plus a currency code.
- Add an index for every list and every lookup, and a unique index for every
  uniqueness rule — the database, not a prior `SELECT`, settles concurrent writes.
- Modules that record events add `autoxide_kernel::event::outbox_migration(...)`; modules
  with background jobs add `autoxide_kernel::job::jobs_migration(...)`.

Entities use SeaORM (`src/entity.rs`). Read with `Database::read()`, write with
`Database::write()`, and group writes in `Database::begin_write()`. Keep write
transactions short: no network calls, no password hashing inside.

### 3. Permissions

Declare them in `src/permissions.rs` as `<module>.<action>` and return them from
`Module::permissions`. Check permissions, never role names. Roles are made by
administrators from the permissions every module declares.

### 4. Service

`src/service.rs` holds every rule. It takes an `Actor` (who acts; from
`Actor::from(&principal)`) so that events record who did it. Each write records its
event in the same transaction with `Outbox::record(&transaction, &tenant, actor, subject,
&event)` — the change and its audit entry exist together or not at all. `tenant` is the
`TenantContext` of the customer the data belongs to (`autoxide_kernel::tenancy`).

### 5. REST API and OpenAPI

Routes live in `src/http.rs`, are registered with `OpenApiRouter::routes(routes!(…))`,
and are mounted under `/api/v1/<name>`; a module declares only the tail (`/`, `/{id}`).

- Read JSON bodies with `ValidJson<T>` (`T: Deserialize + Validate + ToSchema`). A field
  the service checks is checked only there (no `#[validate]` on it), and the service
  collects every problem of the input into one `Error::validation(…)`: callers see all
  their mistakes at once.
- Answer with `Data<T>`, `Created::new("/<name>/<id>", …)` (201 with a `Location` header)
  for a new record, `Page<T>` (lists, via `fetch_page` with
  `limit`/`cursor`/`order`) or an `Error` (sent as RFC 9457 Problem Details).
- A record that can be changed carries `updated_at`. Return it with
  `Tagged { version: row.updated_at, data }` (adds `ETag`) from its `GET` and its update;
  the update handler takes `if_match: IfMatch` and passes `if_match.expected()?` to the
  service, which adds `updated_at = <expected>` to its `UPDATE` and answers
  `version::changed()` (412) when no row matched (see `autoxide_kernel::version`).
- A handler that needs a signed-in caller takes `principal: Principal` and calls
  `principal.require(&PERMISSION)?`. A public endpoint (anyone may call it, signed-in
  callers may see more) takes `principal: Option<Principal>` and declares
  `security((), ("bearer" = []))`.
- To make `If-Match` mandatory on an update, call `if_match.required()?` instead of
  `expected()?` (missing header: 428).
- Lists sorted by something other than creation time use `fetch_page_by` (keyset
  pagination on `(column, id)`, with an index on both); text search uses
  `autoxide_kernel::search` on a lower-cased column. A `GET` that returns an `ETag`
  answers `304` to a matching `If-None-Match` without extra code.
- Stored content is read with `Storage::get_range` for byte ranges, and a module lists
  only its own keys with `Storage::keys_with_prefix("<name>/")`. The `media` module is
  the full example of these (`modules/media`).
- Document every handler: a `///` line (the summary), the required permission, and a
  `description` for every response with the error `code`s. Doc comments on handlers and
  schema types are English, written for API callers.
- Declare who may call each endpoint in `#[utoipa::path]`, right after `path`:
  `extensions(("x-permissions" = json!(["notes.write"])))` (one of the listed permissions is
  enough; `[]` means any signed-in caller) or `extensions(("x-public" = json!(true)))`. The
  description or the 403 response must name the same permissions. The gate checks every
  endpoint: no token → 401, a caller without the permission → 403 `permission_denied`. Check
  the permission first in the handler, before reading the input, so a caller without it
  never gets a 422. An MCP tool either `.requires(…)` a permission or says
  `.for_any_signed_in()`; a read-only tool must be callable with a `read`-level permission.
- A permission's risk (`read`, `write`, `destructive`, `sensitive_data`, `money`) follows from
  its name (`….read` is `read`, others `write`); set another with
  `Permission::new(…).with_risk(Risk::Destructive)`.
- Breaking change: add a new endpoint and declare the old one in `Module::deprecations`.
  The gate (`cargo xtask api-diff`) compares `docs/openapi.json` with the last release and
  refuses breaking changes, naming each one and the way to do it right.
- MCP tools are a contract too, kept in `docs/mcp-tools.json`
  (`cargo run -p autoxide -- mcp-manifest > docs/mcp-tools.json`). For a breaking change add
  a new tool (e.g. `create_note_v2`) and declare the old one with
  `Deprecation::tool("create_note", "YYYY-MM-DD").sunset(…).successor("create_note_v2")`
  in `Module::deprecations`: agents see it in `tools/list` and in every result.

### 6. Events and the audit log

Declare events in `src/events.rs` and return them from `Module::events`. The payload is
`{"before": …, "after": …}` (omit what does not apply) and never contains secrets. The
`audit` module records every event; other modules subscribe through
`Module::subscriptions` and must accept receiving the same event twice.

### 7. MCP tools

`Module::tools` returns `autoxide_kernel::mcp::Tool`s. Each tool takes an input type
(`Deserialize + Validate + ToSchema`, plain fields), requires the same permission as the
matching REST endpoint, and calls the same service method:

```rust
Tool::new("create_note", "Create a note.", move |principal: Principal, input: CreateNoteRequest| {
    let service = service.clone();
    async move {
        let note = service.create(Actor::from(&principal), input.into()).await?;
        Ok::<_, Error>(NoteResponse::from(note))
    }
})
.requires(NOTES_WRITE)
```

Mark tools that only read with `.read_only()` and tools that destroy data with
`.destructive()`. Use `PageInput` for list tools.

Everything a module declares — permissions, events, subscriptions, jobs, schedules, tools,
deprecated endpoints, migrations — shows up in the installed-module catalog at
`GET /api/v1/kernel/modules` (permission `kernel.modules.read`). Nothing extra to write.

### 8. Working with other modules

Publish a trait in `src/api.rs` and provide it in `Module::start` with
`context.provide::<dyn Trait>(…)`. Another module gets it with
`context.require::<dyn Trait>()` and is registered after you. The core's own shared
services work the same way: `dyn Mailer` (send mail; module `mail`) and `dyn Storage`
(store files; module `files`).

### Records other modules attach things to

Modules such as `comments` attach data to records of other modules without depending on them
(`autoxide_kernel::resource`):

- `context.resource_type("shop.item", resolver)` in `Module::start` lets others ask whether an
  item exists and is public, and get its title, path and picture (`Resource::image`). Implement
  `resolve_many` (many ids in one query, titles in a language) when others look up your items in
  batches, e.g. website menus; a type with a tree (categories) returns `true` from `has_children`
  and lists sub-items in `children`. All three have defaults, so older resolvers keep working.
- `Module::resource_events` declares which of your events (from `Module::events`, subject
  `<kind>:<id>`) mean `ResourceChange::Closed` (takes nothing new) or `ResourceChange::Removed`
  (gone: close and hide what is attached). The kernel checks them at start.
- `context.badge_source(kind, &["seller"], resolver)` computes badges of an author on a record of
  any kind, e.g. `seller` for someone who manages a product. Each badge of a kind comes from one
  module; badges are stable codes, never sentences.
- `context.posting_verifier(…)` (one module in the whole program) answers whether an account is
  verified to post content (phone or identity); `context.posting_verification()` asks it. With no
  verifier registered every signed-in account counts as verified.

### Personal data

A module that keeps data about users takes part in data export and account deletion
(see `autoxide_kernel::privacy`):

- implement `PersonalData` (return the user's data as JSON, without secrets) and register
  it in `Module::start` with `context.personal_data(…)`; it then appears in
  `GET /api/v1/auth/me/export` under the module's name;
- subscribe to `privacy::USER_DELETED` in `Module::subscriptions` and delete the user's
  rows when it arrives (`privacy::deleted_user(event)` gives the id). It may arrive twice.

### Money and balances

A module that keeps amounts of money, balances or transactions uses the tools in the
kernel; `crates/kernel/tests/money.rs` shows all of them on a sample wallet table, on both
databases.

- **Amounts are `autoxide_kernel::money::Money`**: an `i64` count of the currency's minor
  unit (cents for `USD`, đồng for `VND`) plus an ISO 4217 `Currency`. Never `f32`/`f64` or a
  database decimal type. `checked_add`, `checked_sub`, `checked_mul` and `checked_neg`
  return an error instead of overflowing; mixing two currencies is an error
  (`money.currency_mismatch`). Split an amount with `allocate(&[ratios])` or `split(n)`:
  the parts always add up to the whole (100 split 3 ways is 34, 33, 33).
- **Store** a `Money` in two columns, `<name>_minor` (`BIGINT`) and `<name>_currency`
  (3 characters); read it back with `Money::from_columns`. **In JSON** it is
  `{"amount": "1999", "currency": "USD"}` — `amount` is a string, because JavaScript
  rounds numbers above 2^53. Use `Money` directly in request and response types; its
  OpenAPI schema says this.
- **Change a balance with one conditional update**, never "read, check, write":
  `rows::adjust::<Entity, _>(&transaction, Column::BalanceMinor, Column::Id.eq(id), -amount,
  Bounds::NON_NEGATIVE)` returns `0` when the balance is not enough (or the row is gone)
  and never lets the column overflow. Concurrent requests queue on the row; the balance
  never goes below the floor.
- **When you must read before you write** (check a status, move between two rows), lock
  the rows first with `rows::lock_one` / `rows::lock` inside `Database::begin_write()`.
  Lock several rows **in one consistent order** — `rows::lock(…, Column::Id)` locks by id —
  so two transactions never wait for each other in a circle (deadlock).
- **Retry on conflict**: wrap the whole write in `Database::write_with_retry(move || async
  move { … })`. It runs the closure again when PostgreSQL reports a deadlock or a
  serialization failure, or SQLite stays busy; any other error returns at once. The
  closure opens its own transaction and must be safe to run again: no mail, no network
  calls, no changes outside the database inside it.
- **A client reference stops duplicate commands for good.** The `Idempotency-Key` header
  only lives 24 hours. For a command with lasting effect (a payment, a transfer) accept a
  `reference` from the caller, check it with `reference::Reference::checked`, and store it
  with the record next to `reference::fingerprint(&content)` under a unique index on
  (owner, `reference`). Run the command with `reference::submit_once`, calling
  `reference::replayed` on the stored record first: the same reference with the same
  content returns the earlier record (`Submitted::Replayed`, answer `200`), with different
  content it fails with `reference_reused` (422), and concurrent duplicates create one
  record.
- **Record the event in the same transaction** as the change (`Outbox::record`), so the
  audit log and other modules see every movement. The audit log is a hash chain:
  `autoxide doctor` reports the first entry that was changed or removed.
- **Never update or delete a transaction.** Correct a mistake with a new, reversing
  transaction (`amount.checked_neg()`) that points to the one it reverses; balances are
  then always explained by the history.

### 9. Configuration, jobs, limits

- Settings: declare each with `Setting` and read it in `Module::configure` through
  `ConfigReader` (never `std::env::var`); names start with `AUTOXIDE_<NAME>_`. Then
  regenerate `.env.example`: `cargo run -p autoxide -- config --env-example > .env.example`.
- Background work: `Module::jobs` (handlers), `Module::schedules` (recurring jobs), and
  `context.jobs()` to enqueue.
- A route that needs a larger request body than the default declares it in
  `Module::body_limits`.
- Settings an interface needs (languages, upload limits…) go in `Module::client_config`: a
  `ClientSection::new(&value)` of a `Serialize + ToSchema` struct with plain fields. They appear
  under the module's name in `GET /api/v1/kernel/client-config`, which needs no sign-in — put
  only values safe to show anyone, never a secret.
- Anything an installer can get wrong (a directory that is not writable, an external
  service that does not answer, missing initial data) gets a check in `Module::checks`:
  a `Check` whose `Probe` returns a `Finding` (ok, warning or failed, with a hint). It runs
  in `autoxide doctor`; it must not change data. See `modules/files/src/doctor.rs`.
- Work an operator runs by hand now and then (rebuild derived data after a setting
  changed, a one-off clean-up) is a task in `Module::tasks`: a `Task` with its
  `TaskArgument`s and a `TaskRun`, run with `autoxide task <module>.<task> name=value`. It
  calls the service, like REST does. See `modules/media/src/images/refresh.rs`.

### 10. Translations

Every error `code` and mail template key has a Vietnamese text in `locales/vi.json`,
returned from `Module::translations`. The English text stays in the code where the
error is made.

### 11. Tests

Tests that touch the database use `TestDatabase::run(async |test| { … })` from
`autoxide-testkit`: every test gets an empty database, SQLite or PostgreSQL depending
on `TEST_DATABASE_URL`, and the gate runs both.

`TestApp` builds your module the way the binary does — configuration, migrations,
`start`, routes, MCP tools and every HTTP layer of the core — and calls it:

```rust
let app = TestApp::builder(test)
    .setting("AUTOXIDE_NOTES_SOMETHING", "42")         // like an environment variable
    .provide::<dyn Storage>(Arc::new(MemoryStorage::default())) // stand-in for another module
    .user("ada", 0xada, &["notes.read", "notes.write"]) // token "ada" → these permissions
    .module(crate::module())
    .build()
    .await
    .unwrap();

let answer = app.post("/api/v1/notes").token("ada").json(json!({"title": "Hi"})).send().await.unwrap();
assert_eq!(answer.status(), StatusCode::CREATED);

let client = app.mcp("ada").await.unwrap();             // a real MCP client
let result = client.call_tool(tool("notes.list_notes", json!({}))).await.unwrap();
```

`app.database()`, `app.modules()` and `app.api::<dyn Trait>()` give access to the rest.
`modules/notes/src/tests.rs` is the full example. Anything written concurrently (unique
names, single-use tokens, counters) needs a concurrency test.

### 12. Finish

1. If the API changed, regenerate the contract:
   `cargo run -p autoxide -- openapi > docs/openapi.json`.
2. While you work, `./gate.sh --quick` runs the tests of your module (and of the crates that
   depend on it) on SQLite in under a minute.
3. Run the full `./gate.sh` until it is green: every test on SQLite and PostgreSQL.

## A module in its own repository

A module does not have to live in this repository. Keep it in its own crate and
depend on the core by a release tag:

```toml
[dependencies]
autoxide-kernel = { git = "https://github.com/<owner>/autoxide", tag = "vX.Y.Z" }

[dev-dependencies]
autoxide-testkit = { git = "https://github.com/<owner>/autoxide", tag = "vX.Y.Z" }
```

Use the Rust version of that release (`rust-toolchain.toml`) or newer, and the same
major versions of the public dependencies (table below), or the compiler reports two
copies of the same type. To run it, add the crate to a checkout
of Autoxide (`crates/app/Cargo.toml` and `crates/app/src/modules.rs`, step 1).

`cargo xtask check` only runs inside this repository, so a module kept elsewhere follows
its rules by hand: the two public items, the table prefix, no foreign key to another
module, every route through `routes!`, translated error codes. Keep the table-ownership
test (`autoxide_testkit::schema_ownership_violations`) and `TestApp` tests on both
databases.

## Stability of the module contract

What a module builds on is the **module contract**: the public items of
`autoxide-kernel` and `autoxide-testkit`, and the libraries whose types appear in them.

| Library | Version | Seen in |
| --- | --- | --- |
| `axum` | 0.8 | handlers, extractors |
| `sea-orm`, `sea-orm-migration` | 2 | entities, `Database`, `Module::migrations` |
| `utoipa` | 6 | `ToSchema`, `#[utoipa::path]` |
| `utoipa-axum` | 0.3 | `Module::routes` (`OpenApiRouter`) |
| `tokio` | 1 | async runtime |
| `serde`, `serde_json` | 1 | bodies, events, tool arguments |
| `uuid` | 1 (v7) | ids |
| `chrono` | 0.4 | times |
| `async-trait` | 0.1 | traits called through `dyn` |
| `validator` | 0.21 | `ValidJson<T>` |

Rules the core follows:

- While the version is `0.x`, a breaking change to the contract comes only in a minor
  release (`0.7` → `0.8`) and is listed under **Changed** or **Removed** in
  `CHANGELOG.md`. A new major version of a library in the table is such a change.
- `Module` only grows by functions with a default body: a module written for an older
  release keeps compiling.
- Public types that will grow are `#[non_exhaustive]`: build them with their
  constructor, not a struct literal, and match their enums with a `_` arm.
- A public item that is going away is `#[deprecated]` (with what to use instead) for at
  least one release before it is removed.

## Checklist

- [ ] Crate added to the workspace and registered in `crates/app/src/modules.rs`
- [ ] Tables named `<module>_…`, UUIDv7 ids, indexes for every list and lookup,
      migrations with `down`
- [ ] Permissions declared; every handler and tool requires one
- [ ] Logic only in the service; each write records its event in the same transaction
- [ ] Every route documented (summary, permission, every response)
- [ ] MCP tools for what agents should be able to do
- [ ] Error codes prefixed and translated
- [ ] Data about users exported (`PersonalData`) and deleted on `USER_DELETED`
- [ ] Money as `Money`; balances changed with `rows::adjust`, rows locked in id order,
      lasting commands deduplicated by `reference`
- [ ] Tests on both databases, including concurrent writes
- [ ] `docs/openapi.json` and `.env.example` regenerated; gate green
