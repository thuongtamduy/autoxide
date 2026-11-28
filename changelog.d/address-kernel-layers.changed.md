- Kernel `db`: `Find::page_by(sort, sort_of, id, request)` pages a list ordered by a NOT NULL column (code, name,
  position, time) then by ID, with the same keyset cursor as `fetch_page_by`; the sealed trait `db::SortKey` names
  the column types it accepts (text, whole numbers, `Timestamp`), so a nullable sort column does not compile.
- Kernel `http`: `PublicCache<T>` wraps a result of a public read with `Cache-Control: public, max-age=…` and an
  optional `ETag`, without naming `axum` in module code.
- Test kit: `autoxide_testkit::control_plane(database, module)` gives the control-plane handle a module gets at
  start, for tests that build a service by hand.
- Module `address` moved to the kernel's layers (manifest, `db` records, `#[endpoint]` routes, kernel `Validate`);
  it no longer depends on `axum`, `sea-orm`, `utoipa`, `validator` or `async-trait`. Behaviour, error codes and the
  OpenAPI document are unchanged.
