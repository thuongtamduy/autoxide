- CLI: `autoxide backup` / `restore` print the absolute path of the backup, explain a permission
  error with the fix (create the directory, give it to uid 10001), and `backup` warns when run in a
  container with a target that is not on a volume or mount. `serve` and `user create` on a database
  that is not migrated now say how many migrations are pending (first few names) and which command
  to run, with the stable error code `kernel.migrations_pending`, instead of listing every
  migration.
