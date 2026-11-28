- Kernel `db`: every statement a tenant handle (`tenant.db()`) runs is checked before it reaches the database, on
  SQLite and Postgres: a statement that touches a tenant table without `tenant_id = <the handle's tenant>` (or an
  `INSERT` without the tenant's `tenant_id`) is refused with `internal_error`, logged with the table name and counted
  in `kernel_tenant_guard_blocked_total`. This covers the query builder, hand-written `handle.sql(…)` (a `:tenant`
  that only sits in a comment no longer passes) and SeaORM statements run on a tenant handle.
