- Operators: `autoxide migrate up --all` (and the API run of placement migrations) no longer stops at the first tenant
  database that fails: every placement is tried, each failure is named, and the command exits with code 1. The
  failed placement stays `behind` (its tenants get 503) until a later run succeeds. The API run now records the
  real cause for each failed placement instead of a generic "internal error".
- `AUTOXIDE_MIGRATE_ON_START=true` now migrates every tenant database too, not only the main one; a failed tenant
  database is logged and the server still starts. `compose.yml` and the Kubernetes manifest run `migrate up --all`.
