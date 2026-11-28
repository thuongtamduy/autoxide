- Kernel `db`: every statement a tenant handle (`tenant.db()`) runs is checked before it reaches the database, on
  SQLite and Postgres. Each use of a tenant table (every table with a `tenant_id` column that is not a declared
  control-plane table) — each `FROM` / `JOIN`, subquery, `UNION` branch, self-join side — needs its own
  `tenant_id = <the handle's tenant>`; an `INSERT` needs the tenant's `tenant_id`, and `ON CONFLICT … DO UPDATE`
  a tenant filter on the update. A refused statement never runs: `internal_error`, logged with the table name,
  counted in `kernel_tenant_guard_blocked_total`. This covers the query builder, hand-written `handle.sql(…)`
  (a `:tenant` that only sits in a comment no longer passes) and SeaORM statements run on a tenant handle.
