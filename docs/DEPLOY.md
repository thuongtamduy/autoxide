# Installing and deploying Autoxide

[Tiếng Việt](DEPLOY.vi.md)

This guide takes you from nothing to a running server you can keep up to date and back
up. Pick one of the two ways to run it:

| | Docker Compose with PostgreSQL | One binary with SQLite |
| --- | --- | --- |
| Good for | Production, several people writing at once, several servers | A small installation on one machine, trying Autoxide |
| You need | Docker with the Compose plugin | Rust (to build) or a built binary; nothing else |
| Database | PostgreSQL 16 or newer, in a container | A file, `data/autoxide.sqlite` |

You can start with SQLite and move to PostgreSQL later without losing data (see
[Moving from SQLite to PostgreSQL](#moving-from-sqlite-to-postgresql)).

## A. Docker Compose with PostgreSQL

### 1. Get the code and set the secrets

```bash
git clone <repository URL> autoxide
cd autoxide
```

Create a file named `.env` next to `compose.yml` with three secrets. Never commit it.

```bash
printf 'POSTGRES_PASSWORD=%s\nAUTOXIDE_AUTH_TOKEN_SECRET=%s\nAUTOXIDE_SECRET_KEY=%s\n' \
  "$(openssl rand -hex 24)" "$(openssl rand -base64 48 | tr -d '\n')" \
  "$(openssl rand -base64 48 | tr -d '\n')" > .env
chmod 600 .env
```

- `POSTGRES_PASSWORD` protects the database.
- `AUTOXIDE_AUTH_TOKEN_SECRET` signs access tokens. Keep it stable: changing it signs
  everyone out.
- `AUTOXIDE_SECRET_KEY` signs short-lived tokens (forms, anti-spam), keys the hashes of
  visitors' IP addresses and encrypts the secrets the server keeps (two-step sign-in, webhook
  signing secrets). Kept outside the database, a copy of the database (a backup) reveals
  neither those addresses nor those secrets.
- To change `AUTOXIDE_SECRET_KEY` later: put the new value in it and the old one in
  `AUTOXIDE_SECRET_KEY_PREVIOUS`, restart, run `autoxide secrets reseal`, then remove
  `AUTOXIDE_SECRET_KEY_PREVIOUS`. Setting it for the first time on a server that ran without it
  needs no extra step: the key the server kept in the database still opens what it encrypted.

### 2. Start

```bash
docker compose up -d --build
```

Or skip building and use a released image (see the project's GitHub releases):

```bash
echo 'AUTOXIDE_IMAGE=ghcr.io/<owner>/autoxide:<version>' >> .env
docker compose up -d --no-build
```

Released images run on `linux/amd64` and `linux/arm64` (for example AWS Graviton or a
Raspberry Pi 4 and newer). `<version>` is a release such as `0.12.0`, or `0.12` to follow its
bug fixes. Each image is built by the project's `Release` workflow on GitHub and carries a
signed build provenance and a signed list of its components (SBOM). Before you run an image,
check that it was built from this repository with the
[GitHub CLI](https://cli.github.com/):

```bash
gh attestation verify oci://ghcr.io/<owner>/autoxide:<version> --owner <owner>
```

It prints the workflow, commit and tag the image was built from, and fails if the image
was not built by this repository. The SBOM (SPDX) is attached to each GitHub release as
`sbom.spdx.json`; check that it belongs to the image and read it with:

```bash
gh attestation verify oci://ghcr.io/<owner>/autoxide:<version> --owner <owner> \
  --predicate-type https://spdx.dev/Document/v2.3 --format json \
  | jq '.[0].verificationResult.statement.predicate.packages[].name'
```

Compose starts PostgreSQL, applies every migration once (the `migrate` service), then
starts the server on `127.0.0.1:8080`. Check it:

```bash
curl http://127.0.0.1:8080/health/ready
```

`{"data":{"status":"ok"}}` means the server and its database are ready.

### 3. Create the first administrator

```bash
docker compose run --rm app user create --email you@example.com --name "Your Name" --admin
```

A strong password is printed once; store it in your password manager. Sign in with
`POST /api/v1/auth/login`; the whole API is described at
`http://127.0.0.1:8080/api/v1/openapi.json`.

### 4. Put it on the internet

The server speaks plain HTTP on `127.0.0.1:8080` only. Put a reverse proxy in front that
terminates TLS. With [Caddy](https://caddyserver.com), which gets certificates by
itself, the whole configuration (`/etc/caddy/Caddyfile`) is:

```text
api.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

Then tell Autoxide which header carries the real client address, so that rate limiting
counts clients and not the proxy. Add to `.env` and restart:

```bash
echo 'AUTOXIDE_HTTP_CLIENT_IP_HEADER=X-Forwarded-For' >> .env
docker compose up -d
```

**More than one proxy.** Each proxy adds the address that connected to it at the end of
`X-Forwarded-For`; whatever comes before was sent by the client and can be forged. Set
`AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` to the number of your own proxies a request passes
through (default `1`). A common case is a website server (such as Next.js) that calls the API
for its visitors, behind a CDN:

```text
browser ──▶ CDN ──▶ website server ──▶ Caddy ──▶ Autoxide
            adds     adds the CDN       adds the website
            browser  address            server address
X-Forwarded-For:  browser, CDN, website-server
                  ▲ 3rd from the end → AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT=3
```

The website server must forward the `X-Forwarded-For` it received (and add the CDN address,
as proxies do); otherwise every visitor looks like one client. With a count, none of your
proxies may be reachable directly from the internet, or clients can choose their address.

**Proxies the internet can reach.** When a visitor could skip the CDN and call your website
server directly, or requests take different paths, a count is wrong for some of them. List
the addresses of your proxies instead, as single addresses and ranges:

```bash
echo 'AUTOXIDE_HTTP_TRUSTED_PROXIES=10.0.0.0/8, 198.51.100.0/24' >> .env
```

This replaces `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT`. Autoxide then reads the header only when
the connection comes from one of these addresses, and takes the last address in the header
that is not one of them. Your CDN publishes its address ranges; keep the list up to date when
they change.

The startup log also warns when `AUTOXIDE_SECRET_KEY` is not set (see the table below): set it
before going live.

Add other settings the same way: every setting is listed, with its default, in
[`.env.example`](../.env.example). Settings `compose.yml` does not pass on yet go under
`environment:` of the `x-autoxide` block.

### 5. Update

```bash
git pull
docker compose up -d --build
```

The `migrate` service applies new migrations before the new server starts.

### 6. Back up and restore

```bash
mkdir -p backups && sudo chown 10001 backups
docker compose run --rm -v "$PWD/backups:/backups" app backup "/backups/$(date +%F)"
```

The backup is a directory holding a copy of the database (as a SQLite file) and of every
stored file; it can be taken while the server runs. Copy it somewhere else — another
machine or object storage. To restore into a fresh installation, start PostgreSQL
alone, restore (`--no-deps` keeps Compose from applying migrations first: restore needs
an empty database), then start everything:

```bash
docker compose up -d postgres
docker compose run --rm --no-deps -v "$PWD/backups:/backups" app restore /backups/2026-11-11
docker compose up -d
```

Restore only backups you made yourself.

## B. One binary with SQLite

### 1. Build

Install Rust from [rustup.rs](https://rustup.rs) (the right version is picked from
`rust-toolchain.toml`) and a C compiler — SQLite and libwebp are compiled into the binary
(on Debian or Ubuntu: `sudo apt install build-essential`) — then:

```bash
cargo build --release --locked -p autoxide
sudo install -m 755 target/release/autoxide /usr/local/bin/autoxide
```

### 2. Prepare a directory

```bash
sudo useradd --system --home-dir /var/lib/autoxide --create-home autoxide
sudo -u autoxide sh -c 'cd /var/lib/autoxide && printf "AUTOXIDE_AUTH_TOKEN_SECRET=%s\nAUTOXIDE_SECRET_KEY=%s\n" "$(openssl rand -base64 48 | tr -d "\n")" "$(openssl rand -base64 48 | tr -d "\n")" > .env && chmod 600 .env'
```

Autoxide reads `.env` from the directory it runs in and keeps everything it writes in
`data/` there: the database `data/autoxide.sqlite`, uploaded files `data/files`, and mail
`data/mail` until SMTP is configured.

### 3. Migrate and create the first administrator

```bash
cd /var/lib/autoxide
sudo -u autoxide autoxide migrate up
sudo -u autoxide autoxide user create --email you@example.com --name "Your Name" --admin
```

Lost the device of your only administrator account with two-step sign-in on? Turn it off from
the server, then sign in with the password and turn it on again:

```bash
sudo -u autoxide autoxide user reset-mfa --email you@example.com
```

Lost the devices holding its passkeys? Remove them from the server, then sign in with the
password and add new ones. Other administrators can do the same through the API with the
permission `auth.passkeys.manage` (`DELETE /api/v1/auth/users/{id}/passkeys`).

```bash
sudo -u autoxide autoxide user remove-passkeys --email you@example.com
```

These three commands act on the default tenant. With several tenants
(`AUTOXIDE_TENANCY_MODE=multi`), add `--tenant <slug>` to act on the account of another tenant,
for example its first administrator:

```bash
sudo -u autoxide autoxide user create --tenant acme --email owner@acme.example --name "Owner" --admin
```

### 4. Run as a service

`/etc/systemd/system/autoxide.service`:

```ini
[Unit]
Description=Autoxide
After=network-online.target
Wants=network-online.target

[Service]
User=autoxide
WorkingDirectory=/var/lib/autoxide
ExecStart=/usr/local/bin/autoxide serve
Restart=on-failure
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/autoxide

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now autoxide
curl http://127.0.0.1:8080/health/ready
```

Put a reverse proxy in front as in [A.4](#4-put-it-on-the-internet) and set
`AUTOXIDE_HTTP_CLIENT_IP_HEADER` in `.env`.

### 5. Update

Build the new version, stop the service, migrate, start:

```bash
sudo systemctl stop autoxide
sudo install -m 755 target/release/autoxide /usr/local/bin/autoxide
(cd /var/lib/autoxide && sudo -u autoxide autoxide migrate up)
sudo systemctl start autoxide
```

### 6. Back up and restore

```bash
cd /var/lib/autoxide
sudo -u autoxide autoxide backup "backups/$(date +%F)"
```

To restore on a new machine, prepare the directory (steps 1–2), then run
`autoxide restore <backup directory>` there before starting the service.

With `AUTOXIDE_TENANCY_MODE=multi`, the backup holds the main database (the tenant register and
every tenant on it) and all files of the shared file storage, but not the databases of other
placements nor a placement's own file storage; the command prints a warning naming them. Back
those up with the database's (and the storage's) own tools. Keep
`AUTOXIDE_SECRET_KEY` in a safe place apart from the backups: without it, the encrypted secrets in
a backup cannot be read.

## Moving from SQLite to PostgreSQL

Create an empty PostgreSQL database, stop the server, and copy. The target gets every
migration first, then all data in one transaction:

```bash
sudo systemctl stop autoxide
cd /var/lib/autoxide
printf '%s' 'postgres://autoxide:<password>@db.example.com:5432/autoxide' \
  | sudo -u autoxide autoxide copy --to-stdin
```

Then set `DATABASE_URL` in `.env` to that URL and start the server. Files stay where
they are; to move them to S3 as well, take a backup, set the S3 settings, and restore
into a new database.

## Settings worth knowing

| Setting | What for |
| --- | --- |
| `DATABASE_URL` | `sqlite://data/autoxide.sqlite` (default) or `postgres://user:password@host:5432/db` |
| `AUTOXIDE_AUTH_TOKEN_SECRET` | Signs access tokens; required in production, the same on every server |
| `AUTOXIDE_SECRET_KEY` | Signs short-lived tokens of every module and keys the hashes of visitors' IP addresses; optional: when unset, one is generated once and kept in the database. Set it (outside the database, the same on every server) in production: with the key in the database, anyone holding a database backup can open the secrets stored encrypted and work the IP blocks back out of the stored hashes |
| `AUTOXIDE_SECRET_KEY_PREVIOUS` | Old values of `AUTOXIDE_SECRET_KEY`, separated by commas, while rotating the key; remove them after `autoxide secrets reseal` |
| `AUTOXIDE_HTTP_CLIENT_IP_HEADER` | Behind a reverse proxy: the header with the client address |
| `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` | How many of your proxies a request passes (default `1`) |
| `AUTOXIDE_HTTP_TRUSTED_PROXIES` | Instead of a count: the addresses and ranges of your proxies, e.g. `10.0.0.0/8, 192.0.2.7` |
| `AUTOXIDE_HTTP_PUBLIC_URL` | The address the internet reaches the API at, e.g. `https://api.example.com`; needed for links back to the API in mail (newsletter unsubscribe) |
| `AUTOXIDE_HTTP_CORS_ORIGINS` | Web pages on other origins allowed to call the API |
| `AUTOXIDE_HTTP_HSTS_SECONDS` | Once every client reaches the API over HTTPS: send `Strict-Transport-Security` (e.g. `31536000`, one year); `autoxide doctor` reminds you when the public URL is `https://` and this is `0`. Every response already carries `nosniff`, `Referrer-Policy`, `X-Frame-Options` and a strict `Content-Security-Policy` (`AUTOXIDE_HTTP_SECURITY_HEADERS`, `AUTOXIDE_HTTP_REFERRER_POLICY`) |
| `AUTOXIDE_MAIL_SMTP_URL`, `AUTOXIDE_MAIL_FROM` | Send mail (password reset, email verification) |
| `AUTOXIDE_FILES_S3_BUCKET` and `AWS_…` | Store files in S3 or an S3-compatible service; also add a bucket lifecycle rule that aborts incomplete multipart uploads after a day |
| `AUTOXIDE_AUTH_PASSWORD_RESET_URL`, `AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL` | Link mails to your own pages |
| `AUTOXIDE_AUTH_PASSKEY_RP_ID`, `AUTOXIDE_AUTH_PASSKEY_ORIGINS` | Turn on passkey sign-in for your domain |
| `AUTOXIDE_HTTP_DRAIN_SECONDS` | Behind a load balancer: how long to keep serving while `/health/ready` says `503` on shutdown, see [below](#updating-without-dropping-requests) |
| `AUTOXIDE_PROCESS_ROLE` | `api` / `worker` to run the API and background work in separate processes |
| `AUTOXIDE_HTTP_METRICS` | Prometheus metrics at `/metrics`, on the API port: when you turn it on, block `/metrics` in your reverse proxy (Caddy: `respond /metrics 404`) and scrape it from the machine itself |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Send traces to an OpenTelemetry collector |

Check the configuration without starting anything: `autoxide config`. It lists every
setting with its value (secrets hidden) and reports every problem at once.

## Settings and state of each tenant

With several tenants (`AUTOXIDE_TENANCY_MODE=multi`), the settings above apply to every tenant.
Some behaviour of the modules can also be set per tenant, replacing the server's value for that
tenant only:

| Tenant setting | Replaces |
| --- | --- |
| `kernel.public_url` | `AUTOXIDE_HTTP_PUBLIC_URL` in links sent by mail |
| `mail.connection.smtp.url`, `mail.connection.smtp.from` | `AUTOXIDE_MAIL_SMTP_URL`, `AUTOXIDE_MAIL_FROM` |
| `ai.connection.text.api_key`, `ai.connection.text.model` | the key and model of the server's AI provider (`AUTOXIDE_AI_API_KEY`, `AUTOXIDE_AI_MODEL`) |
| `comments.banned_words`, `comments.suspect_words`, `comments.max_links`, `comments.guest_moderation`, `comments.user_moderation` | the same `AUTOXIDE_COMMENTS_…` settings ([comments](COMMENTS.md#per-tenant)) |
| `forms.retention_days` | `AUTOXIDE_FORMS_RETENTION_DAYS` ([forms](FORMS.md#settings)) |
| `content.locales`, `content.default_locale` | `AUTOXIDE_CONTENT_LOCALES`, `AUTOXIDE_CONTENT_DEFAULT_LOCALE` ([content](CONTENT.md#languages)) |
| `media.alt_locales` | `AUTOXIDE_MEDIA_ALT_LOCALES` ([media](MEDIA.md)) |

`GET /api/v1/kernel/settings` lists them with what the current tenant set. A tenant's
administrator (permission `kernel.settings.manage`) sets one with
`PUT /api/v1/kernel/settings/{name}` and `{"value": "…"}`, and removes it (back to the server's
value) with `DELETE`. Platform staff do the same for any tenant under
`/api/v1/kernel/tenants/{id}/settings` (`kernel.tenants.read` to list, `kernel.tenants.manage` to
change). Values are checked when they are set; secret ones are stored encrypted and never shown
again. Other servers use a new value within `AUTOXIDE_TENANCY_CACHE_SECONDS`.

Settings that connect a tenant to a partner service (its mail server, its AI provider account…)
are in the group `connection` and are named `<module>.connection.<kind>.<field>`:
`GET /api/v1/kernel/settings?group=connection` lists only those, which is what a "partner
connections" screen needs. Every setting in the list carries its `group` (`null` when it has
none).

The connection settings of mail and AI were renamed to this pattern. Their former names
(`mail.smtp_url`, `mail.from`, `ai.api_key`, `ai.model`) still work: a value a tenant set under a
former name keeps applying, and `PUT` / `DELETE` accept the former name. The list shows the
current names only.

A tenant can be stopped in two ways, independent of each other:

- **Suspended** — `autoxide tenant suspend <tenant>` (back with `resume`), or `PATCH
  /api/v1/kernel/tenants/{id}` with `{"status": "suspended"}`: every request to the tenant, reads
  included, answers `403 tenant_suspended`, and its domains get no new certificates.
- **Read-only** — `autoxide tenant read-only <tenant>` (back with `writable`), or `{"read_only":
  true}`: reads work, its users can still sign in, reset their password, export their data and use
  the links in mails (confirm an address, unsubscribe); every other change answers `403
  tenant_read_only`, and so do MCP tools that are not read-only. Use it, for example, while a
  customer has not paid.

Background jobs and events of either kind of tenant keep running: they finish work that was
accepted before, and deletions after the retention times keep happening. The platform tenant can
be neither suspended nor read-only.

`GET /api/v1/kernel/tenants` (platform staff) takes filters for a control panel — `status`,
`read_only`, `placement_kind`, `schema_state`, and `subscription_status` / `near_limit`, which
match no tenant until the plans module (subscriptions and limits) is installed — and
`expand=placement` to get each tenant's placement in the same answer.

## Custom domains of tenants

With several tenants (`AUTOXIDE_TENANCY_MODE=multi`), each tenant can serve its site on its own
domain, such as `shop.example`: a request to a verified domain reaches that tenant without the
`Autoxide-Tenant` header. An installation with one tenant does not need this: its domain is the
platform's.

1. An administrator of the tenant (permission `kernel.domains.manage`) adds the domain:

   ```bash
   curl -X POST https://api.example.com/api/v1/kernel/domains \
     -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
     -d '{"domain": "shop.example"}'
   ```

   The answer has `"status": "pending"` and a `record` to create at the domain's DNS provider: a
   TXT record named `_autoxide-challenge.shop.example` with the value `autoxide-verify=…`.
2. At the DNS provider, create that TXT record and point the domain itself at your server (an
   `A` / `AAAA` record, or a `CNAME` to the platform's domain).
3. Verify: `POST /api/v1/kernel/domains/{id}/verify` looks the record up at once and answers
   `verified`, or still `pending` with the reason in `check_error`. Pending domains are also
   checked every 10 minutes; one not verified within 7 days becomes `failed` (fix the record,
   then verify it again). `DELETE /api/v1/kernel/domains/{id}` removes a domain.
4. Keep the TXT record. A verified domain is checked again once a day: when the record is missing
   at 3 checks in a row, the domain becomes `failed` (event `kernel.domain_failed`) and requests
   to it no longer reach the tenant. `failed_checks` counts the misses; one check that finds the
   record (or `POST …/verify` after fixing it) resets it to 0. A DNS server that does not answer
   does not count as a miss.

The platform's own domain (the host of `AUTOXIDE_HTTP_PUBLIC_URL`), IP addresses and `localhost`
are refused. A domain belongs to one tenant at a time: another tenant can claim it only after it
was removed or failed.

A domain can belong to one node of the tenant's organization tree — a store with its own site,
for example: `PATCH /api/v1/kernel/domains/{id}` with `{"org_id": "<id of the node>"}` attaches
it, `{"org_id": null}` detaches it (the domain then stands for the tenant as a whole). The node
must be one of the tenant's own; while no installed module keeps an organization tree, only
`null` is accepted. Attaching does not change the domain's status or its DNS record; a removed
domain answers `409 tenant_domain_revoked`. Each change records the event
`kernel.domain_updated`.

**TLS certificates.** Let Caddy get a certificate when the first visitor of a domain arrives
(on-demand TLS), after asking Autoxide whether the domain is allowed:
`GET /api/v1/kernel/domains/allowed?domain=…` answers `200` only for a verified domain of an
active tenant, so nobody can make your server request certificates for names you do not serve.

```text
{
    on_demand_tls {
        ask http://127.0.0.1:8080/api/v1/kernel/domains/allowed
    }
}

api.example.com {
    reverse_proxy 127.0.0.1:8080
}

https:// {
    tls {
        on_demand
    }
    reverse_proxy 127.0.0.1:8080
}
```

Caddy's `reverse_proxy` passes the original `Host` header on, and that is how Autoxide tells the
tenant. When another proxy in front rewrites `Host`, it must send the visitor's domain in
`X-Forwarded-Host` (or `Forwarded: host=…`), and its addresses must be listed in
`AUTOXIDE_HTTP_TRUSTED_PROXIES`: Autoxide reads those headers only from trusted proxies. The
`ask` endpoint counts toward the per-address rate limit like any other; Caddy asks once per new
certificate. Verifying, removing a domain or suspending its tenant reaches every server within
`AUTOXIDE_TENANCY_CACHE_SECONDS` (default `10`).

## Accounts of a tenant: staff, customers, delegation

Each tenant has two kinds of accounts. **Staff** (`realm: staff`, the default) work with roles and
permissions. **Customers** (`realm: customer`) are the accounts of the tenant's shop: they sign in,
have no permission, and are separate from staff even with the same email. Customers register with
`POST /api/v1/auth/register` and sign in with `POST /api/v1/auth/login`, both with
`"realm": "customer"`; a tenant closes customer sign-up with the organization setting
`auth.customer_registration` (`false`). Staff manage them with `GET /api/v1/auth/customers` and
`PATCH /api/v1/auth/customers/{id}` (permissions `auth.customers.read`, `auth.customers.manage`).

**Inviting staff.** Instead of creating accounts with passwords, invite people:
`POST /api/v1/auth/invitations` with an `email`, the `role_ids` and the unit (`org_id`) they get
(permission `auth.invitations.manage`; you cannot invite with roles whose permissions you lack),
or from the command line:

```bash
autoxide user invite --email new.colleague@example.com --role editor
```

The mail carries a one-time link to `AUTOXIDE_AUTH_INVITATION_URL` (a page of your interface with
`{token}`; unset: the mail carries the token), valid `AUTOXIDE_AUTH_INVITATION_DAYS` days (7).
Accepting creates the account (or, when the email already has one, adds the roles after the person
signs in). **Signing in with a link** (no password) is off by default; turn it on with
`AUTOXIDE_AUTH_LOGIN_LINK=true` and set `AUTOXIDE_AUTH_LOGIN_LINK_URL` to your interface's page.
Both need working mail (`AUTOXIDE_MAIL_SMTP_URL`) and a running worker. The organization tree of
a tenant is at most `AUTOXIDE_ORGANIZATIONS_MAX_DEPTH` levels deep (8 by default). Import the list of
administrative units once, for checking addresses: `autoxide task address.import`
([`docs/ADDRESS.md`](ADDRESS.md)).

**Platform staff and a tenant's data.** Platform staff (accounts of the platform tenant) never
enter a tenant's data just because of their role: someone with `auth.support.grant` grants one
staff member time-limited access with `POST /api/v1/auth/support-grants` (`tenant_id`,
`staff_user_id`, a platform `role_id`, `basis` `support` or `legal_request` with its document
number, a `reason`, at most `AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS` hours, 8 by default). The
tenant sees every grant, its first use and every change made (`GET /api/v1/auth/support-access`),
can revoke a running `support` grant (`DELETE /api/v1/auth/support-access/{id}`), and can turn
support access off with the setting `auth.support_access` (then only `legal_request` grants work).

In a chain of tenants (a tenant with `parent_id`), the parent can let holders of one of its roles
work in a child: `POST /api/v1/auth/delegations` with `child_tenant_id`, `role_id`, optional
`org_id` (a unit of the child) and `expires_at`, and a `reason` (permission
`auth.delegations.manage`). Those people keep their own sign-in and add the header
`Autoxide-Tenant: <child id>`; `GET /api/v1/auth/me/tenants` lists where they can go. Every change
they make in the child is recorded with the delegation as the way in (`actor_access` in the
child's events and audit log); revoking (`DELETE /api/v1/auth/delegations/{id}`) takes effect at
their next request. The child sees what it received with
`GET /api/v1/auth/delegations?direction=incoming`.

## Moving a tenant to another database

With several tenants, each tenant's data lives on a *placement*: `main` (the database of
`DATABASE_URL`, shared by many tenants) or another database added with `autoxide placement add`.
Move a tenant to give a growing customer a database of its own, or to bring it back:

1. Add the target and apply the migrations to it:

   ```bash
   printf '%s' 'postgres://autoxide:<password>@db2.example.com:5432/acme' \
     | autoxide placement add acme-db --kind dedicated --url -
   autoxide migrate up --all
   ```

2. Try it first. A dry run changes nothing:

   ```bash
   autoxide tenant move acme --to acme-db --dry-run
   ```

   It lists the rows of the tenant that would be copied, per table, and everything that blocks
   the move: migrations not applied on one side, a target that is the same database as the
   source, a tenant that is not active, another move of the tenant. The platform tenant
   (`default`) always stays on `main`.

3. Move, in a maintenance window:

   ```bash
   autoxide tenant move acme --to acme-db
   ```

   From the start, the tenant answers `503 tenant_maintenance` with `Retry-After`; other tenants
   are not affected. The command waits until every server has seen the move
   (`AUTOXIDE_TENANCY_CACHE_SECONDS`) and requests in progress are done
   (`AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS`), and until jobs and events of the tenant already
   being run have finished. Then it copies every row of the tenant, queued jobs and undelivered
   events included, in one transaction; compares the row counts of every table; switches the
   tenant to the new placement (it works again from there); and deletes the old copy. SQLite and
   PostgreSQL can be mixed in either direction. Files (under `tenants/<id>/`) stay where they are
   when both placements use the same file storage; when the target has its own storage (below),
   they are copied there before the rows and deleted from the old storage at the end. The dry run
   shows how many files would be copied.

`--keep-source` keeps the old copy as a backup instead of deleting it (its queued jobs and events
are deleted, so nothing runs twice; its files stay too). Nothing uses it; delete it once you are
satisfied:

```bash
autoxide tenant purge-copy acme --placement main
```

Over the API, platform staff with `kernel.tenants.move` call
`POST /api/v1/kernel/tenants/{id}/move` with `{"to": "acme-db"}`: that is a dry run. Add
`"dry_run": false` to start the move as a background job, and follow it at the `Location` of the
answer (`GET /api/v1/kernel/tenants/{id}/moves/{move_id}`); the MCP tool is `kernel.move_tenant`.
A job may run for about four minutes: move large tenants (the dry run shows the row count) with
the command, which has no time limit.

**A placement's own file storage.** By default every tenant's files are kept in the storage of
the `files` module settings (`AUTOXIDE_FILES_DIR` or `AUTOXIDE_FILES_S3_BUCKET`), under
`tenants/<id>/`. Give a placement — typically a `dedicated` one — its own bucket or directory, so
a customer's files live apart too:

```bash
printf '%s' "$SECRET_ACCESS_KEY" | autoxide placement storage acme-db \
  --s3-bucket acme-files --s3-region eu-west-1 \
  --s3-endpoint https://<account>.r2.cloudflarestorage.com \
  --s3-access-key-id "$ACCESS_KEY_ID" --s3-secret-access-key -
# or a directory on the server:
autoxide placement storage acme-db --dir /var/lib/autoxide/files-acme
```

The settings are stored encrypted with `AUTOXIDE_SECRET_KEY`; `autoxide placement list` and
`GET /api/v1/kernel/placements` show the bucket or directory, never the keys. The bucket must
exist; a wrong key shows up at the first upload. Set the storage while the placement has no
tenants, then move tenants onto it: the move copies their files. Changing it later is refused
(their files are in the old storage), except with `--force` when the new storage already holds
the same files — for example a new access key for the same bucket. `--clear` sends the files of
new uploads back to the shared storage (same rule). Every server must run the `files` module.

**When a move stops half-way** (the process dies, a database cannot be reached, row counts
differ), nothing is lost. Before the switch the tenant keeps answering `503` on its old placement;
after it, the tenant works from the new placement and only the old copy waits to be deleted.

- Run the same `autoxide tenant move acme --to acme-db` again: it resumes where it stopped
  (before the switch it deletes the rows it had copied to the target and copies again).
- Or, before the switch, give up: `autoxide tenant move acme --abort` deletes what was copied and
  makes the tenant active again on its old placement.
- `GET /api/v1/kernel/tenants/{id}/moves/{move_id}` shows the `state` of the move and the `error`
  that stopped it. A move started over the API is retried by its job; starting another run while
  one is running answers `tenant_move_running`.

## Running several servers

With PostgreSQL, run as many `app` processes as you need behind the reverse proxy: they
share nothing but the database (and S3 for files), and background jobs are claimed so
that each runs once. Set the same `AUTOXIDE_AUTH_TOKEN_SECRET` everywhere. To scale the
API and the background work separately, run some processes with
`AUTOXIDE_PROCESS_ROLE=api` and at least one with `worker`. SQLite allows one server
process.

### Updating without dropping requests

When a server is told to stop (`SIGTERM`, as Docker, systemd and Kubernetes send), it
goes through two phases:

1. **Drain** for `AUTOXIDE_HTTP_DRAIN_SECONDS` (default `0`): it keeps handling every
   request, but `/health/ready` answers `503` with code `shutting_down`, so your load
   balancer stops sending it new requests. `/health/live` stays `200`: the process must
   not be restarted.
2. **Stop**: it stops accepting connections and gives requests in progress up to
   `AUTOXIDE_HTTP_SHUTDOWN_SECONDS` (default `30`) to finish.

For a rolling update behind a load balancer:

- Point the load balancer's health check (or the Kubernetes `readinessProbe`) at
  `/health/ready`, and the `livenessProbe` at `/health/live`.
- Set `AUTOXIDE_HTTP_DRAIN_SECONDS` longer than the load balancer needs to take a server
  out: its check interval times the number of failed checks it waits for. A check every
  5 seconds that gives up after 2 failures needs more than 10, so `15`.
- Give the process time for both phases before it is killed: Docker Compose
  `stop_grace_period`, Kubernetes `terminationGracePeriodSeconds`, systemd
  `TimeoutStopSec` must be at least drain + shutdown (`45s` for `15` + `30`). Docker waits
  only 10 seconds by default; the `compose.yml` in this repository sets `40s`, enough for
  no drain — raise it when you set one.
- Start the new servers and wait until their `/health/ready` answers `200` before stopping
  the old ones.

With a single server and no load balancer, keep the drain at `0`: there is nowhere else to
send requests, and the drain would only delay the stop.

## Checklist before going live

- [ ] `AUTOXIDE_AUTH_TOKEN_SECRET` and `AUTOXIDE_SECRET_KEY` set to long random values, kept
      secret (`autoxide doctor` warns when one is missing)
- [ ] TLS in front (reverse proxy), port 8080 not reachable from the internet
- [ ] `AUTOXIDE_HTTP_CLIENT_IP_HEADER` set to your proxy's header
- [ ] SMTP configured, so password reset and email verification work
- [ ] Backups taken on a schedule and copied to another machine; a restore tried once
- [ ] `/health/ready` watched by your monitoring
