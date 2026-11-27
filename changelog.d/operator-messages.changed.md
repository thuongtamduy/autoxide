- `autoxide migrate up` and `down` print one line per module when there are many migrations (a first start has about
  four hundred); `autoxide migrate status` lists each one. `migrate down --steps 0` is refused.
- `backup` and `restore` no longer log a line per table (they print the summary); the per-table lines are at `debug`.
- While the server drains (graceful shutdown), the `/health/ready` answers `shutting_down` are logged at `debug`, not
  `ERROR`; request log lines carry `tenant_id` once.
- A database that cannot be reached (wrong port, server down) is reported with its root cause next to the pool
  timeout; an error in `.env` names the variable (and never echoes the line, which may hold a password).
- The "SMTP is not set" warning is logged when mail is first written to a file, not at the start of every command.
- The error text of `copy`, `backup` and `restore` names the right command (`autoxide backup`, `autoxide restore`).
- `AUTOXIDE_AUTH_TOKEN_SECRET` is described as recommended, not required, in `docs/DEPLOY.md`.
