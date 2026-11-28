- Module `comments` moved to the kernel's layers (manifest, `db` records, `#[endpoint]` routes, kernel `Validate`,
  `time::Duration`); it no longer depends on `axum`, `sea-orm`, `jiff`, `utoipa`, `validator` or `async-trait`.
  Behaviour, error codes, the OpenAPI document and the schema are unchanged.
- Kernel `db`: `Find::page_tiers` (ranked tiers, same cursor as `fetch_page_tiers`), `Find::page_keyed` (page by a
  UUIDv7 column other than `id`, also a nullable one filled on every row), integer arithmetic on `i32` columns,
  `bench_seed::insert_records` for records.
- Kernel `http`: `Routes::allowed_while_read_only()`, `BodyLimit::post`/`put`/`patch`; `version::MaybeTagged`
  sends `ETag` only when the record has a version.
