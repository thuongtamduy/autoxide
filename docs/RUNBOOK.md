# Runbook

**English** · [Tiếng Việt](RUNBOOK.vi.md)

What to do when something goes wrong in production, in the order to do it. Every command and endpoint here exists in
this version (the CLI help, `docs/openapi.json` and the metric names were checked against the code). The per-alert
cases (server errors, slow requests, connections, backups, the last administrator) are in the
[runbook of `DEPLOY.md`](DEPLOY.md#runbook); this page covers the incidents that need a decision and a sequence of steps.

## How to use this page

1. **Contain first, explain later.** Stop the damage (suspend a tenant, revoke a key, take a server out of rotation), then
   find the cause.
2. **Write down what you do and when** (a plain text log in your incident channel). You will need it for the review, and
   for a data breach notice if personal data was exposed ([`DEPLOY.md`](DEPLOY.md#personal-data-breaches)).
3. **Never paste secrets, tokens or personal data** into tickets or chat. Refer to a `request_id` instead; every error
   response carries one.
4. First command for almost everything: `autoxide doctor` (add `--json` for scripts and AI agents). It changes nothing and
   says what is wrong and how to fix it.

In the commands below, `API` is the address of the API, `TOKEN` is an access token of a user with the permission the step
names, and `jq` is installed.

## Before an incident

- Prometheus scrapes `/metrics` (`AUTOXIDE_HTTP_METRICS=true`, own port) and loads
  [`deploy/prometheus/autoxide-alerts.yml`](../deploy/prometheus/autoxide-alerts.yml); the Grafana dashboard
  [`deploy/grafana/autoxide-dashboard.json`](../deploy/grafana/autoxide-dashboard.json) is imported.
- Scheduled backups run (`autoxide doctor` warns when not), are copied off the storage they sit on, and a restore was
  practised on a spare machine ([`DEPLOY.md`](DEPLOY.md#restoring-step-by-step)).
- At least two people can sign in as platform staff, and at least one can run `autoxide` on the server
  (`autoxide user create … --admin` makes a new administrator if every one is locked out).
- The secrets (`AUTOXIDE_SECRET_KEY`, `AUTOXIDE_AUTH_TOKEN_SECRET`) are stored outside the repository and outside the
  database backups, and you know where.

## Database full or slow

Full (writes fail with "no space left on device"): free or add disk space first, then see
[`DEPLOY.md`, "Database full"](DEPLOY.md#database-full). SQLite: look at the size of the database file and its `-wal`
file (a long read such as a backup keeps `-wal` growing). PostgreSQL: the largest tables with
`SELECT relname, pg_size_pretty(pg_total_relation_size(relid)) FROM pg_statio_user_tables ORDER BY pg_total_relation_size(relid) DESC LIMIT 10;`.
Finished jobs and delivered events are removed after 7 days by themselves.

Slow (alert `AutoxideSlowP95`, requests waiting): check `database_connections{state="in_use"}` against
`database_connections_max` (alert `AutoxideDbPoolExhausted`); on PostgreSQL look at `pg_stat_activity` for long queries,
on SQLite remember there is one write connection by design. A query without an index belongs in a new migration of the
module that owns the table (`EXPLAIN ANALYZE` on PostgreSQL, `EXPLAIN QUERY PLAN` on SQLite). Raise
`AUTOXIDE_DATABASE_READ_CONNECTIONS` / `AUTOXIDE_DATABASE_WRITE_CONNECTIONS` only after ending what holds connections,
and keep the total of all processes under PostgreSQL's `max_connections`.

## Background queue backlog

Signs: alerts `AutoxideJobsStuck`, `AutoxideEventsBacklog`, `AutoxideDeadLetterGrowing`; mail, webhooks, exports arrive late.

1. Is a worker running? At least one process must have `AUTOXIDE_PROCESS_ROLE` set to `all` or `worker`; check its
   `/health/ready` and logs.
2. Which module is behind? `jobs_pending` and `jobs_oldest_pending_seconds` by `module` and `database`, or, as platform
   staff with `kernel.ops.read`: `curl -H "Authorization: Bearer $TOKEN" "$API/api/v1/kernel/ops/backlog"`.
3. Multi-tenant: a database being migrated answers `503 tenant_maintenance` and its work waits; let
   `autoxide migrate up --all` finish.
4. Jobs that gave up: `autoxide jobs failed [--module <module>] [--limit <n>]` lists them with the last error. Fix the cause
   (a wrong SMTP password, a partner down) **before** `autoxide jobs retry <module> <id>`; a job retried too early only fails again.
5. Events that gave up: a module that kept failing on one event is retried alone, so the other modules already have
   it. Each row of `kernel_event_deliveries` with `failed_at` set is one module (`subscriber`) that never received one
   event (`module`, `event_id`), with `last_error`. Fix the cause, then run
   `UPDATE kernel_event_deliveries SET failed_at = NULL, attempts = 0 WHERE id = '<id>';` to retry that module only.
   A row of `<module>_events` with `failed_at` set could not be read at all (damaged payload, tenant gone).
6. Still behind with a healthy worker: raise `AUTOXIDE_JOBS_CONCURRENCY` or start more worker processes.

## A key leaked

Decide which key, then rotate it. Do not wait to be sure.

**The access token signing key (`AUTOXIDE_AUTH_TOKEN_SECRET`).** An attacker who has it can forge access tokens.

1. Generate a new value: `openssl rand -base64 48`.
2. Set it on **every** server and restart them. Do **not** put the leaked value in `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS`
   (that variable keeps an old key accepted for a smooth routine change).
3. Every access token stops working at once; clients get a new one with their refresh token, which stays valid. If the
   refresh tokens may be stolen too, also sign users out: see [mass revocation](#mass-revocation-of-api-keys-and-sessions).

**The encryption key (`AUTOXIDE_SECRET_KEY`).** It encrypts stored secrets (two-step sign-in secrets, webhook signing
secrets, provider credentials) and keys the hashes of visitors' IP addresses.

1. Set the new value, and the old one in `AUTOXIDE_SECRET_KEY_PREVIOUS`, on every server; restart.
2. Run `autoxide migrate up` once if you upgraded, then `autoxide secrets reseal`: it encrypts every stored secret again
   with the new key.
3. Remove `AUTOXIDE_SECRET_KEY_PREVIOUS` from every server and restart.
4. Backups made before the rotation still open with the old key: delete them or keep them as safe as the key was. If
   the **database** (or a backup) leaked together with the key, the secrets inside must be treated as exposed: ask the
   tenants to rotate their provider credentials and webhook secrets.

**An API key or a provider credential.** Revoke the key (`DELETE /api/v1/auth/all-api-keys/{id}` for any user's key, with
`auth.users.manage`; users revoke their own with `DELETE /api/v1/auth/api-keys/{id}`); `POST /api/v1/auth/api-keys/{id}/rotate`
replaces one with a new key. A provider credential (payment, SMS, shipping) is changed in the provider's console first,
then in the tenant's settings.

## Mass revocation of API keys and sessions

Use it when accounts or devices may be compromised and you cannot tell which. Containment first:

- **Stop one tenant completely:** `autoxide tenant suspend <id-or-slug>` (every request to it answers 403) or
  `PATCH /api/v1/kernel/tenants/{id}` with `{"status": "suspended", "reason": "…"}` (permission `kernel.tenants.manage`, platform
  staff). Undo with `autoxide tenant resume <id-or-slug>` or `"status": "active"`. For "readable but not changeable" use
  `autoxide tenant read-only <id-or-slug>` or `{"read_only": true}`, undone by `autoxide tenant writable`.
- **Sign one user out everywhere:** `DELETE /api/v1/auth/users/{id}/sessions` (permission `auth.users.manage`). An access
  token already issued keeps working until it expires (15 minutes by default, `AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS`).
  Disabling the account (`PATCH /api/v1/auth/users/{id}`) stops new sign-ins.
- **Sign every user of a tenant out:** there is no single call; loop over the users (the list is paged: `limit` up to 200,
  `cursor` from `pagination.next_cursor`):

  ```bash
  cursor=""
  while :; do
    page="$(curl -sf -H "Authorization: Bearer $TOKEN" "$API/api/v1/auth/users?limit=200${cursor:+&cursor=$cursor}")"
    jq -r '.data[].id' <<<"$page" | while read -r id; do
      curl -sf -X DELETE -H "Authorization: Bearer $TOKEN" "$API/api/v1/auth/users/$id/sessions" >/dev/null
    done
    cursor="$(jq -r '.pagination.next_cursor // empty' <<<"$page")"
    [ -n "$cursor" ] || break
  done
  ```

  Then change `AUTOXIDE_AUTH_TOKEN_SECRET` (see "A key leaked") so access tokens die at once instead of within 15 minutes.
- **Revoke every API key of a tenant:** list them with `GET /api/v1/auth/all-api-keys` (same paging) and call
  `DELETE /api/v1/auth/all-api-keys/{id}` for each; build the loop like the one above. Re-issue the keys that are
  legitimate afterwards (`POST /api/v1/auth/api-keys`).
- **A user who lost a device or a second factor:** `autoxide user reset-mfa` and `autoxide user remove-passkeys` on the
  server, or `DELETE /api/v1/auth/users/{id}/mfa` and `DELETE /api/v1/auth/users/{id}/passkeys` through the API.

Afterwards read the audit log for what the compromised credentials did (see
[a customer reports lost data](#a-customer-reports-lost-data)).

## Restoring one tenant from a backup

There is no command that restores one tenant into a live shared database: it would overwrite newer data of that tenant
with old data and cannot be undone. Choose by how the tenant is stored.

- **The tenant has its own database (its own placement).** Restore that placement's database from its backup as in
  [`DEPLOY.md`](DEPLOY.md#restoring-step-by-step), or from a PostgreSQL point-in-time recovery to the second before the
  mistake. Suspend the tenant first (`autoxide tenant suspend <id-or-slug>`), restore, run `autoxide doctor` and
  `autoxide migrate up --all`, then `autoxide tenant resume <id-or-slug>`.
- **The tenant shares the main database.** Restore the backup into a **temporary installation** (a spare database and
  storage, the same `AUTOXIDE_SECRET_KEY`):

  ```bash
  # on the spare machine, DATABASE_URL and the storage settings point at EMPTY targets
  autoxide restore ./restore-dir        # applies the migrations, copies every table and file
  autoxide doctor
  autoxide serve
  ```

  There, read the tenant's data (the API, or `POST /api/v1/kernel/tenants/{id}/exports`, permission
  `kernel.data.export`, then `GET …/exports/{export_id}/content`), and put back what was lost through the live API: single
  records (`POST`/`PATCH` of the module), a revision (`POST /api/v1/content/entries/{id}/revisions/{revision_id}/restore`),
  an item in the trash (`POST /api/v1/media/{id}/restore`), CSV imports where the module has one (for example
  `POST /api/v1/catalog/imports`). For a large loss, moving the tenant to its own placement
  (`autoxide tenant move … --dry-run` first) gives it a database that can be restored alone next time.
- Afterwards run the module's own checks where they exist (`autoxide task inventory.verify_levels`,
  `autoxide task catalog.rebuild_storefront tenant=<slug>`) and tell the tenant what was restored and what is lost
  (everything changed after the backup).

## A customer reports lost data

1. **Do not change anything yet.** Note the tenant, the record, the time the customer last saw it, and who can act in the
   tenant.
2. **Look for what happened.** The audit log (permission `audit.read`):
   `curl -H "Authorization: Bearer $TOKEN" "$API/api/v1/audit?subject=<record id>&limit=50"`; narrow with `module`,
   `action`, `actor_user_id`, `actor_kind` (a person, an API key or an AI agent). The request log of the tenant
   (`GET /api/v1/kernel/devtools/requests`, permission `kernel.devtools.read`) shows recent signed-in calls for `AUTOXIDE_REQUEST_LOG_DAYS` days (default 14). Search
   the server logs with the `request_id` and with `tenant_id` (see "Monitoring").
3. **It was deleted or changed by someone legitimate:** most deletions can be undone from the module — a content revision
   (`POST /api/v1/content/entries/{id}/revisions/{revision_id}/restore`), a media item in the trash
   (`POST /api/v1/media/{id}/restore`). Otherwise take the old value from the audit entry and write it back through the API.
4. **It was done by a credential that should not have acted:** treat it as a compromise: revoke
   ([mass revocation](#mass-revocation-of-api-keys-and-sessions)), then restore as above.
5. **No trace of a change and the data is simply gone:** suspect the platform: check `autoxide doctor`, the migration
   history (`autoxide migrate status`), recent deployments and manual database access. Restore from a backup or
   point-in-time recovery ([above](#restoring-one-tenant-from-a-backup)).
6. **Tell the customer** what you found, what was restored and what could not be. If personal data was exposed or lost,
   follow [`DEPLOY.md`, "Personal data breaches"](DEPLOY.md#personal-data-breaches) (deadlines apply).

## Monitoring: what to watch and when to alert

Enable `AUTOXIDE_HTTP_METRICS=true` and give `/metrics` its own port (`AUTOXIDE_HTTP_METRICS_ADDRESS`); never expose it to the
internet. With `OTEL_EXPORTER_OTLP_ENDPOINT` set, traces are exported too, carrying `tenant_id` (metrics have no tenant
label by design; look a tenant up in logs and traces).

| Watch (metric)                                            | Healthy                          | Alert (file `autoxide-alerts.yml`)                | Then                         |
| --------------------------------------------------------- | -------------------------------- | ------------------------------------------------- | ---------------------------- |
| `http_requests_total{status=~"5.."}` over all requests    | under 1%                         | `AutoxideHigh5xxRate` above 2% for 10 min         | `DEPLOY.md` "Server errors"  |
| `http_request_duration_seconds` p95 by `route`            | under 0.5 s                      | `AutoxideSlowP95` for 10 min                      | "Database full or slow"      |
| `database_connections{state="in_use"}` / `…_max`          | under 70%                        | `AutoxideDbPoolExhausted` at 90%                  | "Database full or slow"      |
| `jobs_oldest_pending_seconds`, `events_oldest_pending_seconds` | seconds                     | `AutoxideJobsStuck`, `AutoxideEventsBacklog`      | "Background queue backlog"   |
| `jobs_dead`, `events_dead`                                | 0                                | `AutoxideDeadLetterGrowing`                       | `autoxide jobs failed`       |
| `backup_last_success_timestamp_seconds`, `backup_runs_total{outcome="failed"}` | within one interval | `AutoxideBackupTooOld`, `AutoxideBackupFailed` | `DEPLOY.md` "Backups" |
| `http_rate_limited_total`, `http_api_key_rate_limited_total`, `http_tenant_rate_limited_total` | near 0 | `AutoxideRateLimitedClients` above 1/s for 15 min | "Mass revocation" |
| `webhooks_failed_total` vs `webhooks_delivered_total`     | under 5% failing                 | `AutoxideWebhooksFailing` above 20% for 30 min    | the receivers' owners        |
| `mail_bounces_total{kind="hard"}`                         | few                              | `AutoxideMailHardBounces` above 20 per hour       | `DEPLOY.md` "Mail that bounces" |

Other series worth a panel (no alert by default): `database_write_retries_total` (write contention),
`kernel_request_log_dropped_total` (request log overloaded), `entitlement_denials_total` (plan limits hit),
`mcp_tool_calls_total` (agent traffic), `jobs_retried_total`, `backup_size_bytes` and `backup_duration_seconds` (growth).

Thresholds are starting points: watch for a few weeks, then tighten them to what is normal for your deployment. Every
alert has an `action` annotation that says what to do; add routing (severity `page` wakes someone, `warn` goes to a channel)
in Alertmanager.

## After the incident

Within a week write a short review: what happened and when, how it was found, what was done, what it cost, and what
changes so it does not happen again (an alert, a check, a step added to this page). Fix the runbook where reality differed
from it. A vulnerability in Autoxide itself goes through [`SECURITY.md`](../SECURITY.md), not a public issue.
