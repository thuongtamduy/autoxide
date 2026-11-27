- `autoxide rollback --to X.Y.Z` brings the main database and every tenant database back to the migrations of an
  earlier release (0.14.0 or later). `--dry-run` lists, per database, the migrations to revert and the tables and
  columns that would be dropped with how many rows they hold (on SQLite on a copy, on PostgreSQL inside a rolled-back
  transaction bounded by `--lock-timeout-seconds`); a real run needs `--yes`, and `--accept-data-loss` when data is
  dropped. Modules a binary adds itself are left as they are unless `--revert-untracked`. It is recorded as a
  migration run of kind `rollback`.
- `autoxide migrate manifest` prints every migration of the program, without a database.
- `GET /api/v1/kernel/placements/migrations` returns `kind`, `from_version` and `to_version` for each run.
- Module authors: `MigrationTrait::irreversible()` (default `false`) marks a migration a rollback must not cross.
