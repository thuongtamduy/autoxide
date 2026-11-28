- A database migrated by a newer Autoxide than the running binary now fails with its own error,
  `kernel.database_newer`, naming the module and the migrations this binary does not know and how to
  fix it (run the newer binary, or `autoxide rollback --to <this version>` with it); `autoxide serve`
  and `autoxide doctor` report it that way instead of as a broken migration. Kernel:
  `MigrationError::database_newer` returns the new `migration::DatabaseNewer`.
