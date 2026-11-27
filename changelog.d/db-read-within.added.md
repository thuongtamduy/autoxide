- Kernel `db` layer: `within(limit)` runs a read with a time limit, on a query built with `find` (with `all`,
  `one`, `count`, `exists`, `select` and `page`) or on hand-written SQL (`Sql::within`). The database stops the query when it runs past the limit or when
  the caller stops waiting (PostgreSQL `statement_timeout` and `pg_cancel_backend`, SQLite interrupt), and the read
  fails with `503 db.query_timeout` (`db::is_query_timeout`). Waiting for a pool connection counts toward the
  limit. `Database::read_pool_size()` gives the size of the read pool.
