- Kernel `db` layer: `Sql::within(limit)` runs a hand-written read with a time limit. The database stops the
  query when it runs past the limit or when the caller stops waiting (PostgreSQL `statement_timeout` and
  `pg_cancel_backend`, SQLite interrupt), and the read fails with `503 db.query_timeout`
  (`db::is_query_timeout`). `TenantDb::backend()` tells which database a tenant is on.
