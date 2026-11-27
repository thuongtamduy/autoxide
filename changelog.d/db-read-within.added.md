- Kernel `db` layer: `within(limit)` runs a read with a time limit, on a query built with `find` (also after
  `select`) or on hand-written SQL (`Sql::within`). The database stops the query when it runs past the limit or when
  the caller stops waiting (PostgreSQL `statement_timeout` and `pg_cancel_backend`, SQLite interrupt), and the read
  fails with `503 db.query_timeout` (`db::is_query_timeout`).
