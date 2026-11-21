- `autoxide serve` can prepare itself on start, all off by default: `AUTOXIDE_MIGRATE_ON_START=true`
  applies pending migrations, `AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL` (with
  `AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD` or `…_PASSWORD_FILE`, else a generated password logged once)
  creates the first administrator while there is none, and `AUTOXIDE_STARTUP_TASKS` runs maintenance
  tasks such as `address.import,tax.import_pack,promotions.load_legal_data`. Servers starting
  together take turns through a migration lock (a Postgres advisory lock, a lock file next to the
  SQLite file), which `autoxide migrate up` takes too.
- `GET /` answers with the server's name, version and links to the OpenAPI document, the API
  reference (when on), the MCP endpoint, `/llms.txt` and `/health/ready`.
