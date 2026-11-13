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
  database.
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
event in the same transaction with `Outbox::record(&transaction, actor, subject,
&event)` — the change and its audit entry exist together or not at all.

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
- Breaking change: add a new endpoint and declare the old one in `Module::deprecations`.

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

### Personal data

A module that keeps data about users takes part in data export and account deletion
(see `autoxide_kernel::privacy`):

- implement `PersonalData` (return the user's data as JSON, without secrets) and register
  it in `Module::start` with `context.personal_data(…)`; it then appears in
  `GET /api/v1/auth/me/export` under the module's name;
- subscribe to `privacy::USER_DELETED` in `Module::subscriptions` and delete the user's
  rows when it arrives (`privacy::deleted_user(event)` gives the id). It may arrive twice.

### 9. Configuration, jobs, limits

- Settings: declare each with `Setting` and read it in `Module::configure` through
  `ConfigReader` (never `std::env::var`); names start with `AUTOXIDE_<NAME>_`. Then
  regenerate `.env.example`: `cargo run -p autoxide -- config --env-example > .env.example`.
- Background work: `Module::jobs` (handlers), `Module::schedules` (recurring jobs), and
  `context.jobs()` to enqueue.
- A route that needs a larger request body than the default declares it in
  `Module::body_limits`.
- Anything an installer can get wrong (a directory that is not writable, an external
  service that does not answer, missing initial data) gets a check in `Module::checks`:
  a `Check` whose `Probe` returns a `Finding` (ok, warning or failed, with a hint). It runs
  in `autoxide doctor`; it must not change data. See `modules/files/src/doctor.rs`.

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
2. Run `./gate.sh` until it is green.

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
- [ ] Tests on both databases, including concurrent writes
- [ ] `docs/openapi.json` and `.env.example` regenerated; gate green
