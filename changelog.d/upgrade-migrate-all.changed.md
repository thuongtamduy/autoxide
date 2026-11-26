- Operators: `autoxide migrate up --all` (and the API run of placement migrations) no longer stops at the first tenant
  database that fails: every placement is tried, each failure is named, and the command exits with code 1. The
  failed placement stays `behind` (its tenants get 503) until a later run succeeds. The API run now records the
  real cause for each failed placement instead of a generic "internal error".
- Only one process migrates tenant databases at a time, whichever way it starts (the command line, the API run,
  tenant moves): they share a lock of their own, separate from the main database's, so a server that starts never
  waits for a long run over the tenant databases, and a command-line run shows up in the list of migration runs.
  A run left unfinished by a process that stopped is closed as `failed` by the next one instead of blocking every
  new run, and a tenant database it left `migrating` is picked up again.
- `AUTOXIDE_MIGRATE_ON_START=true` now also migrates every tenant database: the main one before the server opens its
  port, the others in the background as a migration run. `compose.yml` and the Kubernetes manifest run
  `migrate up --all`.
