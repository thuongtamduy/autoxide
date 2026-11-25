# Installing and deploying Autoxide

[Tiếng Việt](DEPLOY.vi.md)

This guide takes you from nothing to a running server you can keep up to date and back
up. Pick one of the two ways to run it (a third, one container with SQLite and no build,
is in [section C](#c-one-container-with-sqlite)):

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
git clone https://github.com/thuongtamduy/autoxide.git autoxide
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
- `AUTOXIDE_AUTH_TOKEN_SECRET` signs access tokens. To change it without signing anyone out
  (as a routine, every few months): put the new value in it and the old one in
  `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` on every server, restart them, and remove
  `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` after `AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS` (15 minutes by
  default): tokens signed with the old key are accepted until they expire. Changing it without
  the old key signs every access token out.
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

The first build compiles everything from source: count on roughly 20 minutes to well over an hour,
and at least 4 GB of memory for Docker. Behind a company proxy, give Docker the proxy first (see
[Requirements in the README](../README.md#requirements)).

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

Autoxide compresses JSON and text responses itself (gzip or brotli, whichever the client
accepts; images and other files are sent as they are), so the proxy needs no `encode`
directive. If your proxy or CDN already compresses, set `AUTOXIDE_HTTP_COMPRESSION=false` so the
work is not done twice.

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

**Updating from 0.17 or earlier:** experimental modules (bookings, newsletter, comments, support,
conversations, analytics, reporting, automations) no longer run unless `AUTOXIDE_MODULES` names
them. Nothing is deleted, and the startup log and `autoxide doctor` tell you when the database
holds data of a module that is off, with the line to add — see
[MODULES.md](MODULES.md#upgrading-from-017-or-earlier). This applies to setups B and C too.

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
docker compose up -d --wait postgres
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
# Above AUTOXIDE_HTTP_MAX_CONNECTIONS (4096) plus room for files and database connections.
LimitNOFILE=8192

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

## C. One container with SQLite

For trying Autoxide or a small installation on one machine, without building anything. The
steps to the first run are in the README, [Try it with Docker](../README.md#try-it-with-docker-no-build).
The rules that matter after that, with `IMAGE` and `RUN` as defined there:

- By default the server does not apply migrations by itself. Starting `serve` on an empty or older
  database stops at once with `N migration(s) have not been applied to the database (first: …); run
  autoxide migrate up first` (the count and the first few names, not the whole list; `autoxide
  migrate status` lists them all; error code `kernel.migrations_pending`): run `$RUN $IMAGE migrate up`, then start it.
  On a single server you may set `AUTOXIDE_MIGRATE_ON_START=true` instead: `serve` then applies
  them on start, under a lock so that two servers starting together never both migrate. With
  `AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL` it also creates the first administrator, and with
  `AUTOXIDE_STARTUP_TASKS` it loads reference data — what `compose.quickstart.yml` does. For
  several servers, keep running `migrate up` once per deployment: you see what changes, and a
  failed migration stops the deployment before the new version takes requests.
- Everything the server writes is in the volume `autoxide-data` (`/var/lib/autoxide/data`):
  the database, uploaded files, and mail until SMTP is configured. A container started without
  that volume keeps its data only until it is removed.
- `docker run -p 8080:8080` publishes the port on every network interface of the machine; use
  `-p 127.0.0.1:8080:8080` and a reverse proxy ([A.4](#4-put-it-on-the-internet)). Put more
  settings in `autoxide.env` (one `NAME=value` per line, no quotes).
- Update: pull the new image, apply its migrations, replace the container.

```bash
IMAGE=ghcr.io/thuongtamduy/autoxide:<new version>
docker pull $IMAGE
$RUN $IMAGE migrate up
docker rm -f autoxide
docker run -d --name autoxide --restart unless-stopped --env-file autoxide.env \
  -v autoxide-data:/var/lib/autoxide/data -p 127.0.0.1:8080:8080 $IMAGE serve
```

- Back up to a directory you mount (the image runs as user 10001, which must be able to write
  there), and restore into a new volume before starting the server:

```bash
mkdir -p backups && sudo chown 10001 backups
$RUN -v "$PWD/backups:/backups" $IMAGE backup "/backups/$(date +%F)"
# on the new machine, or with the volume removed:
$RUN -v "$PWD/backups:/backups" $IMAGE restore /backups/2026-11-11
```

Without the `chown`, the backup stops with `permission denied` and says which directory to
give to uid 10001. The command prints the absolute path of the backup when done. A backup written
to a path that is not mounted disappears with the container: inside a container, `backup` warns
on stderr when the target is not on a volume or bind mount (it still runs).

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
| `AUTOXIDE_MODULES` | Which modules run: `official` (default), `core`, `all`, names to add, `-name` to leave out. Experimental modules must be named. See [MODULES.md](MODULES.md) |
| `AUTOXIDE_AUTH_TOKEN_SECRET` | Signs access tokens; required in production, the same on every server |
| `AUTOXIDE_SECRET_KEY` | Signs short-lived tokens of every module and keys the hashes of visitors' IP addresses; optional: when unset, one is generated once and kept in the database. Set it (outside the database, the same on every server) in production: with the key in the database, anyone holding a database backup can open the secrets stored encrypted and work the IP blocks back out of the stored hashes |
| `AUTOXIDE_SECRET_KEY_PREVIOUS` | Old values of `AUTOXIDE_SECRET_KEY`, separated by commas, while rotating the key; remove them after `autoxide secrets reseal` |
| `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` | Old values of `AUTOXIDE_AUTH_TOKEN_SECRET`, separated by commas, while rotating it; only used to check tokens, remove after the access token lifetime |
| `AUTOXIDE_HTTP_CLIENT_IP_HEADER` | Behind a reverse proxy: the header with the client address |
| `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` | How many of your proxies a request passes (default `1`) |
| `AUTOXIDE_HTTP_TRUSTED_PROXIES` | Instead of a count: the addresses and ranges of your proxies, e.g. `10.0.0.0/8, 192.0.2.7` |
| `AUTOXIDE_HTTP_MAX_CONNECTIONS` | Most open connections at once, WebSockets included (default `4096`, `0`: no limit); over it new connections wait. Keep it below the open file limit (`ulimit -n`, often 1024; systemd `LimitNOFILE`, `docker run --ulimit nofile=…`) with some room for database connections and files — the server warns at startup when it is not. Your reverse proxy should still limit connections per client |
| `AUTOXIDE_HTTP_PUBLIC_URL` | The address the internet reaches the API at, e.g. `https://api.example.com`; needed for links back to the API in mail (newsletter unsubscribe) |
| `AUTOXIDE_HTTP_CORS_ORIGINS` | Web pages on other origins allowed to call the API |
| `AUTOXIDE_HTTP_HSTS_SECONDS` | Once every client reaches the API over HTTPS: send `Strict-Transport-Security` (e.g. `31536000`, one year); `autoxide doctor` reminds you when the public URL is `https://` and this is `0`. Every response already carries `nosniff`, `Referrer-Policy`, `X-Frame-Options` and a strict `Content-Security-Policy` (`AUTOXIDE_HTTP_SECURITY_HEADERS`, `AUTOXIDE_HTTP_REFERRER_POLICY`) |
| `AUTOXIDE_MAIL_SMTP_URL`, `AUTOXIDE_MAIL_FROM` | Send mail (password reset, email verification) |
| `AUTOXIDE_FILES_S3_BUCKET` and `AWS_…` | Store files in S3 or an S3-compatible service; also add a bucket lifecycle rule that aborts incomplete multipart uploads after a day |
| `AUTOXIDE_AUTH_PASSWORD_RESET_URL`, `AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL` | Link mails to your own pages |
| `AUTOXIDE_AUTH_PASSKEY_RP_ID`, `AUTOXIDE_AUTH_PASSKEY_ORIGINS` | Turn on passkey sign-in for your domain |
| `AUTOXIDE_HTTP_DRAIN_SECONDS` | Behind a load balancer: how long to keep serving while `/health/ready` says `503` on shutdown, see [below](#updating-without-dropping-requests) |
| `AUTOXIDE_PROCESS_ROLE` | `api` / `worker` to run the API and background work in separate processes |
| `AUTOXIDE_HTTP_COMPRESSION` | `true` (default): compress JSON and text with gzip or brotli; `false` when a reverse proxy or CDN in front already compresses |
| `AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS` | How long a CDN may keep a public read (default `60`), see [A CDN in front of Autoxide](#a-cdn-in-front-of-autoxide) |
| `AUTOXIDE_HTTP_METRICS` | Prometheus metrics at `/metrics`, which has no authentication. Add `AUTOXIDE_HTTP_METRICS_ADDRESS` (e.g. `127.0.0.1:9090`) to serve it on its own port, away from the API; without it, `/metrics` is on the API port: block it in your reverse proxy (Caddy: `respond /metrics 404`) and scrape it from the machine itself |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Send traces to an OpenTelemetry collector |

Check the configuration without starting anything: `autoxide config`. It lists every
setting with its value (secrets hidden) and reports every problem at once.

## A CDN in front of Autoxide

Public reads of a website (`/api/v1/content/public/…`, `/api/v1/catalog/public/…`) answer with
`Cache-Control: public, max-age=0, s-maxage=60, stale-while-revalidate=300`, an `ETag` and
`Vary: Accept-Language, Accept-Encoding, Origin, autoxide-tenant`. A CDN (Cloudflare, CloudFront, Fastly…) keeps
such a response for `AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS` seconds (default `60`; `0` makes it ask every time);
browsers always ask again and get `304 Not Modified` when nothing changed. A changed post reaches visitors
within that many seconds. Media files with `?v=` in the address never change and are kept for a year
(`immutable`).

Autoxide never lets a shared cache keep anything personal: a request with `Authorization` or a cookie gets
`private` (or `no-store`), and draft previews get `no-store`; writes are never cached. Set the CDN to:

- **respect the origin's cache headers** (Cloudflare: Cache Rules → "Use cache-control header if present";
  CloudFront: a cache policy with minimum TTL `0`, honouring `Cache-Control`);
- **never cache a request that has `Authorization`** — Cloudflare and CloudFront skip it by default when the
  origin says `private`; check that no rule overrides it;
- include the **host** in the cache key (it chooses the tenant) and forward `Accept-Language`, `Origin` and
  the `autoxide-tenant` header if your websites send it;
- not compress again if it already does — then set `AUTOXIDE_HTTP_COMPRESSION=false`.

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
| `content.embed_hosts` | `AUTOXIDE_CONTENT_EMBED_HOSTS` |
| `analytics.allowed_hosts` | `AUTOXIDE_ANALYTICS_ALLOWED_HOSTS` (the tenant's verified domains are always accepted) |
| `newsletter.postal_address` | `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`; when unset, the owner's name and address in the tenant's legal profile (module `compliance`) come first |
| `<module>.link.<page>` (group `link`, see below) | the `AUTOXIDE_…_URL` page addresses in mail: `auth.link.password_reset`, `auth.link.email_verification`, `auth.link.invitation`, `auth.link.login`, `auth.link.email_change_report`, `support.link.guest_ticket`, `compliance.link.case_tracking`, `inventory.link.restock_confirm`, `notifications.link.unsubscribe`, `comments.link.verify`, `comments.link.unsubscribe`, `newsletter.link.confirm`, `newsletter.link.preferences` |

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

### Links in mail, per tenant

Mail sent for a tenant links to pages of that tenant's website or app. Two layers, the second wins:

1. `kernel.public_url` (e.g. `https://shop.example`) replaces the scheme and host of every page
   address the server has (`AUTOXIDE_AUTH_PASSWORD_RESET_URL`…), keeping its path.
2. A link setting (group `link`, `GET /api/v1/kernel/settings?group=link`) gives the whole page
   address of one link, path included, for a site whose pages are not where the server's are —
   e.g. `auth.link.password_reset` = `https://shop.example/dat-lai-mat-khau?token={token}`. A
   tenant can also turn on a mail link the server has no address for.

A link setting is accepted only when it is an `https://` address that contains every placeholder
its description names (`{token}`, `{id}`…), with no user name and no placeholder in the host part,
on a host the tenant controls: one of its verified domains (see "Custom domains of tenants") on the
default port, the address (host and port) of its `kernel.public_url`, or the host of
`AUTOXIDE_HTTP_PUBLIC_URL` on the default port. Otherwise `PUT` answers `422` with code
`kernel.link_host_not_allowed`, `kernel.link_placeholder_missing` or `kernel.link_template_invalid`.
The host is checked again every time a mail is sent: when the domain is revoked, fails its DNS
recheck or moves to another tenant, mail falls back to the server's address (with a warning in the
log) rather than sending a token to a host the tenant no longer controls — at once on the server
that saw the change, within `AUTOXIDE_TENANCY_CACHE_SECONDS` (default 10 seconds) on the others. A
verified domain of another tenant is never trusted, and a domain the tenant lost is not trusted
until it is verified again — also for `kernel.public_url`, which then answers `422`
`kernel.public_url_not_allowed`. One gap remains: a `kernel.public_url` host that nobody has ever
verified is trusted for that tenant's own links, so verify the domain you put there.

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
`POST /api/v1/auth/registrations` and sign in with `POST /api/v1/auth/login`, both with
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
**Signing in with Google, Microsoft, GitHub or OpenID Connect** is off by default; turn it on with
`AUTOXIDE_AUTH_OIDC=true`. Each tenant registers its application at the provider and adds it with
`POST /api/v1/auth/oidc-providers` ([`docs/guides/oidc.md`](guides/oidc.md)); client secrets are
stored encrypted with `AUTOXIDE_SECRET_KEY`. The server calls providers only on public addresses;
keep `AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS=false` in production, and let the server reach the
providers over HTTPS (outbound port 443).
Links and invitations need working mail (`AUTOXIDE_MAIL_SMTP_URL`) and a running worker. The organization tree of
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

   Platform staff can also migrate the placements from the API, without a shell on the server:
   `POST /api/v1/kernel/placements/migrations` with `{"dry_run": true}` lists what would run;
   without `dry_run` it starts a run in the background (follow it with
   `GET /api/v1/kernel/placements/migrations/{id}`). `main` is always migrated by
   `autoxide migrate up` before the server starts.

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

**Data residency.** Record where each placement's servers are with `autoxide placement country
<name> VN` (`-` for unknown; `autoxide placement list` shows it). A tenant that must keep its data in
one country (set by platform staff with `PUT /api/v1/compliance/residency`) cannot move to a placement
in another country or of unknown country: the move answers `tenant_residency_violation`. See
[COMPLIANCE.md](COMPLIANCE.md#data-residency).

## Splitting a branch into its own tenant, and merging tenants

A tenant's organization tree can outgrow it: a region becomes a company of its own, or a shop that
had its own tenant joins a chain. Platform staff split a branch into a new tenant on another
database, or merge a tenant into another one under one of its units. Every row keeps its id, so
splitting and merging back gives the original data again.

```bash
# Split unit <north> of `acme` into the tenant `acme-north` on placement `acme-north-db`.
autoxide tenant split acme --org <north> --slug acme-north --name "Acme North" \
  --placement acme-north-db --reason "Contract 2026-11: the region becomes its own company" --dry-run
# Merge `acme-north` back into `acme` under unit <root-or-other-unit>.
autoxide tenant merge acme-north --into acme --under-org <unit> --reason "…" --dry-run
```

- **Dry run first.** It lists, for every table, the rows that move, are copied, stay, or must run
  first (queued events and due jobs), what blocks (a tenant that is not active, another split, merge
  or move in progress, migrations missing, a target that is the same database), and the
  **conflicts**: rows that point across the boundary (a row that goes refers to a row that stays,
  or the other way) and unique values the receiving tenant already has (two pages with the same
  slug). Fix them, then run the same command without `--dry-run`.
- **Two databases.** The target placement must be another database than the source tenant's. To
  split into the shared database, or to merge two tenants of the same database, move one of them
  first (`autoxide tenant move`).
- **What happens.** The tenant (both tenants when merging) answers `503 tenant_maintenance`; its
  queued events and due jobs run first (`AUTOXIDE_RELOCATION_DRAIN_SECONDS`, default 300). Then the
  rows are copied in one transaction on the target, keeping their ids; the new tenant gets the id
  of the branch's root unit, which becomes its root. Rows are checked, the tenants switch in one
  transaction (domains attached to units of the branch go with it; a merged tenant becomes
  `merged`), and the rows that left are deleted from the source. Files of what moved are copied
  under the new tenant's prefix. The audit log of what moved is written again into the target's
  hash chain; the source keeps its own chain untouched. Ledger books move with their unit, and their
  seal chain still verifies. The report store of the tree follows (`reporting.rehome`).
- **What stays.** Each module declares how its tables move (`Module::relocation`): personal data of
  the tenant (subscribers, consents, identity checks, comments) stays with the source when
  splitting and moves when merging; staff accounts with a role in the branch are copied (they sign in
  again); roles and similar settings are copied; subscriptions, sessions and counters stay.
- **Legal basis.** Moving data between two tenants can transfer personal data between two
  controllers; `--reason` is required and kept with the run and in the audit log.
- **Resume.** If it stops half-way, run `autoxide tenant split --resume <id>` (or `merge`); every
  step can run again. `autoxide tenant relocation-status <id>` shows the state and the error.

Over the API, platform staff with `kernel.tenants.relocate` call `POST /api/v1/kernel/relocations`
(`{"kind": "split", "tenant": "acme", "org_id": "…", "slug": "…", "name": "…", "placement": "…",
"reason": "…"}`): that is a dry run; add `"dry_run": false` to run it as a background job and follow
it at `GET /api/v1/kernel/relocations/{id}`. A tenant cannot split or merge itself: its
administrators (`kernel.relocations.request`) send a request with
`POST /api/v1/kernel/tenant/relocation-requests`, which lands in the work inbox of the platform staff,
who run it with `"request_id"`.

## Personal data breaches

When personal data may have leaked (a stolen backup, an exposed database, a wrong mail sent), the report
to the authority is due **72 hours after detection** in Vietnam. Record the breach in the API at once
(`POST /api/v1/compliance/incidents`), set the tenant policy `incident.alert_emails` so reminders reach
people, and follow the runbook in [COMPLIANCE.md](COMPLIANCE.md#personal-data-breaches-the-72-hour-runbook).
`autoxide doctor` warns when a breach is past its deadline.

## Running several servers

With PostgreSQL, run as many `app` processes as you need behind the reverse proxy: they
share nothing but the database (and S3 for files), and background jobs are claimed so
that each runs once. Set the same `AUTOXIDE_AUTH_TOKEN_SECRET` everywhere. To scale the
API and the background work separately, run some processes with
`AUTOXIDE_PROCESS_ROLE=api` and at least one with `worker`. SQLite allows one server
process.

Limits are counted per process by default: with N servers a client address or an API key gets
up to N times its limit. To share the limit per API key between servers, set
`AUTOXIDE_HTTP_RATE_LIMIT_STORE=database`: every server counts in the database, taking requests
in small lots (a quarter of `AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_BURST`), so a key gets at most one
lot per server more than its limit each minute. The limit per address stays per process.

## Kubernetes

[`deploy/kubernetes/autoxide.yaml`](../deploy/kubernetes/autoxide.yaml) is a minimal, commented
example: a migration `Job`, a `Deployment` of two pods, a `Service` and an `Ingress` with HTTPS. It
follows the rules of [Running several servers](#running-several-servers):

- PostgreSQL runs outside the cluster file (a managed database or your own operator); SQLite allows
  one process and does not fit here.
- Files go to S3 (`AUTOXIDE_FILES_S3_BUCKET`) and mail to SMTP (`AUTOXIDE_MAIL_SMTP_URL`): pods keep
  nothing on disk, so any pod can be replaced.
- Secrets (`DATABASE_URL`, `AUTOXIDE_AUTH_TOKEN_SECRET`, `AUTOXIDE_SECRET_KEY`, the S3 keys, the SMTP
  URL) live in a Kubernetes `Secret` you create with `kubectl create secret`, never in the file.
- Migrations run once per release in the `Job`; pods refuse to start while one is pending, so the
  old pods keep serving until the `Job` has finished.
- `readinessProbe` on `/health/ready`, `livenessProbe` on `/health/live`, a drain of 15 seconds and
  `terminationGracePeriodSeconds: 50` give rolling updates without dropped requests (see
  [Updating without dropping requests](#updating-without-dropping-requests)).
- `AUTOXIDE_HTTP_RATE_LIMIT_STORE=database` shares the per-key limit between pods, and
  `AUTOXIDE_HTTP_CLIENT_IP_HEADER` with one trusted proxy reads the client's address that the
  ingress controller passes on.
- The pods run as the image's non-root user (`10001`) without extra privileges.

The commands to create the secret, apply the file and update are at the top of the file. To split
the API from the background work, copy the `Deployment` into a second one with
`AUTOXIDE_PROCESS_ROLE=worker` (no `Service`, at least one pod) and set `AUTOXIDE_PROCESS_ROLE=api`
on the first.

## Uploads straight to S3

With files on S3, clients can upload large files straight to the bucket
(`POST /api/v1/files/uploads`, guide: [`docs/guides/direct-uploads.md`](guides/direct-uploads.md)).
Browsers need the bucket to allow it (CORS). On Amazon S3, for example:

```json
[{"AllowedOrigins": ["https://admin.example.com"], "AllowedMethods": ["PUT"],
  "AllowedHeaders": ["content-type", "x-amz-checksum-sha256"], "MaxAgeSeconds": 3600}]
```

The server checks every completed upload by reading it back (size and SHA-256), because not every
S3-compatible service checks the signed checksum; raise `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS`
if you allow very large files. Abandoned uploads are removed by an hourly job.

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

## Scheduled backups

Besides `autoxide backup` run by hand, every process that runs background work backs up every
database it handles each `AUTOXIDE_BACKUP_INTERVAL_HOURS` hours (default 24; `0` turns this off),
the same way `autoxide backup` does, into the file storage of the `files` module (the directory or
the S3 bucket) under `kernel/backups/<database>/<id>/`. Only the newest `AUTOXIDE_BACKUP_KEEP` good
backups of each database are kept (default 7). Each backup is a `backup.json`, a `database.sqlite`
and, with `AUTOXIDE_BACKUP_INCLUDE_FILES=true`, the stored files under `files/`.

- The backups sit on the same storage as the uploads, so they protect against lost or broken data,
  not against losing that storage: copy the bucket or directory (backups and uploads) to another
  place, for example with the bucket's replication or a nightly `rclone sync`. That is also why the
  files are not copied into each backup by default.
- To restore one, copy its directory out of the storage (for S3:
  `aws s3 cp --recursive s3://<bucket>/kernel/backups/main/<id>/ ./restore/`; for a directory,
  `cp -r`), then restore that directory as in "Restoring, step by step" below.
- Each run is recorded in the table `kernel_backups` (status, size, error). `autoxide doctor` warns
  when a database has no good backup from the last two intervals, and `/metrics` has
  `backup_last_success_timestamp_seconds`, `backup_runs_total`, `backup_duration_seconds` and
  `backup_size_bytes` (alerts `AutoxideBackupTooOld`, `AutoxideBackupFailed`).
- A backup holds personal data. Data erased on someone's request stays in the backups made before,
  until they are rotated out (`AUTOXIDE_BACKUP_KEEP` × interval): say so in your privacy notice.
  Keep the storage private, in the country where the tenants' data must stay, and keep
  `AUTOXIDE_SECRET_KEY` apart from it. Accounting records that must be kept for 10 years live in the
  `invoices` and `ledger` modules, not in rotated backups. (Legal details: check the original texts
  or someone qualified.)
- While a backup of a SQLite database runs, its `-wal` file grows until the backup ends: plan disk
  space for it.

### Restoring, step by step

Practise this before you need it: the test `dien_tap_backup_restore_roi_khoi_dong` does the same on
every gate run.

1. **Get the backup directory.** From a scheduled backup, copy `kernel/backups/<database>/<id>/` out
   of the file storage (S3: `aws s3 cp --recursive s3://<bucket>/kernel/backups/main/<id>/ ./restore/`;
   a directory: `cp -r`). Take the newest row with status `succeeded` in `kernel_backups`, or the
   newest directory. From `autoxide backup`, use its directory as it is. Only restore backups you
   made yourself.
2. **Prepare an empty target**: a new, empty database (`createdb` on PostgreSQL, or a path to a
   SQLite file that does not exist yet) and an empty file storage. Use the same
   `AUTOXIDE_SECRET_KEY` as when the backup was made, or the encrypted secrets in it cannot be read.
   A backup taken from PostgreSQL can be restored into SQLite and the other way round.
3. **Stop the servers** that point at the target, then run `autoxide restore <directory>` with
   `DATABASE_URL` and the file storage settings of the target. It applies the migrations of this
   version, copies every table and file, and refuses a database that already has Autoxide tables.
4. **Start and check**: `autoxide doctor`, then `autoxide serve`; `/health/ready` must answer `200`
   and an administrator must be able to sign in. Run `autoxide migrate up --all` first in
   multi-tenant mode.
5. **Multi-tenant mode**: a backup holds one database. Restore each placement's database the same
   way. To get back the data of a single tenant, restore the backup into a temporary installation
   and read or export that tenant's data from there; there is no command that restores one tenant
   into a live database.

## Encrypted off-site backups and point-in-time recovery

`autoxide backup` and the scheduled backups are consistent snapshots: restoring one brings the data
back as it was when it was taken, and what happened after is lost. Two additions cover the rest.

### Encrypting a backup before it leaves the machine

A backup holds every module's data, personal data included. Encrypt it before copying it to another
machine or a bucket, for example with [age](https://age-encryption.org) and a key pair whose private
half is kept offline:

```bash
age-keygen -o backup-key.txt            # once; keep backup-key.txt offline, apart from the backups
age-keygen -y backup-key.txt            # prints the public key, age1…, used below

autoxide backup "backups/$(date +%F)"
tar -C backups -c "$(date +%F)" | age -r age1… > "autoxide-$(date +%F).tar.age"
# copy autoxide-<date>.tar.age off-site, then delete the clear directory

# to restore: decrypt, then restore the directory as in "Restoring, step by step"
age -d -i backup-key.txt autoxide-2026-11-21.tar.age | tar -x -C restore/
```

Keep `AUTOXIDE_SECRET_KEY` apart from the backups as well: without it the encrypted secrets inside a
backup cannot be read, which is the point.

### Point-in-time recovery on PostgreSQL

To get the database back as it was at any second — just before a wrong bulk change, for example —
PostgreSQL must keep its write-ahead log (WAL) next to regular base backups. Autoxide needs nothing
special for this: it is the standard PostgreSQL setup.

- **Managed PostgreSQL** (Amazon RDS, Google Cloud SQL, Azure, …): turn on automated backups with
  point-in-time recovery and storage encryption in the provider's console; restoring creates a new
  database at the time you choose. Point `DATABASE_URL` at it.
- **Your own server**: [pgBackRest](https://pgbackrest.org) keeps base backups and WAL in a bucket,
  encrypted on the database server before they are sent. A sketch (check the pgBackRest user guide
  of your version; on Debian or Ubuntu, `apt install pgbackrest`; with the `compose.yml` PostgreSQL
  container, use an image that adds it):

  ```ini
  # /etc/pgbackrest/pgbackrest.conf
  [global]
  repo1-type=s3
  repo1-s3-bucket=<bucket>
  repo1-s3-endpoint=s3.<region>.amazonaws.com
  repo1-s3-region=<region>
  repo1-path=/pgbackrest
  # client-side encryption: a long random value, kept apart from the bucket
  repo1-cipher-type=aes-256-cbc
  repo1-cipher-pass=<openssl rand -base64 48>
  repo1-retention-full=2

  [autoxide]
  pg1-path=/var/lib/postgresql/16/main
  ```

  ```ini
  # postgresql.conf
  wal_level = replica
  archive_mode = on
  archive_command = 'pgbackrest --stanza=autoxide archive-push %p'
  ```

  The S3 keys go in `PGBACKREST_REPO1_S3_KEY` and `PGBACKREST_REPO1_S3_KEY_SECRET` (or
  `repo1-s3-key`, `repo1-s3-key-secret` in a file only `postgres` reads). Then, as the `postgres`
  user:

  ```bash
  pgbackrest --stanza=autoxide stanza-create
  pgbackrest --stanza=autoxide check
  pgbackrest --stanza=autoxide --type=full backup   # weekly, from cron
  pgbackrest --stanza=autoxide --type=diff backup   # daily, from cron
  ```

  To go back to a point in time: stop the Autoxide servers and PostgreSQL, then

  ```bash
  pgbackrest --stanza=autoxide --delta --type=time --target-action=promote \
    "--target=2026-11-21 09:59:00+00" restore
  ```

  start PostgreSQL, run `autoxide doctor`, then start the servers.

Point-in-time recovery covers the database only. Files in S3 are not rolled back with it: turn on
versioning of the bucket to get deleted or overwritten files back. In multi-tenant mode each
placement is its own database and needs its own setup. Practise a restore before you need one, as
for snapshots.

## Monitoring

Turn on `AUTOXIDE_HTTP_METRICS=true`. `/metrics` has no authentication, so give it its own port with
`AUTOXIDE_HTTP_METRICS_ADDRESS` — `127.0.0.1:9090` on a machine, `0.0.0.0:9090` in a container or a
Kubernetes pod (publish that port to Prometheus only, never to the internet) — and point Prometheus
at it; `/metrics` then leaves the API port. Without it, `/metrics` is served on the API port: block
it in the reverse proxy and scrape it from the machine itself. Besides the request figures (`http_requests_total`,
`http_request_duration_seconds`), every process that runs background work (`AUTOXIDE_PROCESS_ROLE`
`all` or `worker`) updates the backlog gauges every `AUTOXIDE_METRICS_BACKLOG_SECONDS` (default 30),
labelled by `database` and `module`:

| Gauge | Meaning |
|---|---|
| `jobs_pending` | Jobs not done yet, scheduled ones included |
| `jobs_oldest_pending_seconds` | How long the oldest job that is due has waited (0 when none) |
| `jobs_dead` | Jobs in the dead-letter queue (`autoxide jobs failed`) |
| `events_pending`, `events_oldest_pending_seconds`, `events_dead` | The same for events waiting to be delivered to subscribers |
| `database_connections{pool,state}`, `database_connections_max{pool}` | Connections of the `read` and `write` pools, `in_use` or `idle`, and their maximum |

The `database` label names the database (`main`, or the placement of a tenant with its own database);
past `AUTOXIDE_METRICS_MAX_DATABASE_LABELS` (default 50) databases, the others are added up under
`database="other"`. There is no label per tenant: look a tenant up in the logs and traces.

**One tenant.** Metrics have no tenant label (a label per tenant would make Prometheus series grow
without bound). To look into one tenant:

- **Logs and traces.** Every request line carries `tenant_id` once the tenant is known (in JSON logs
  under `span.tenant_id`), and so do the spans of background jobs (`job`), event deliveries
  (`event_delivery`) and MCP tools (`mcp_tool`); with `OTEL_EXPORTER_OTLP_ENDPOINT` the traces carry the
  same `tenant_id` attribute. Filter by it: `jq 'select(.span.tenant_id == "<id>")'`, or
  `tenant_id="<id>"` in your log or trace search.
- **Usage.** `GET /api/v1/plans/tenants/{tenant_id}/usage` (module `plans`) gives the tenant's use of
  each limit.
- **Backlog.** `GET /api/v1/kernel/ops/backlog` (platform staff, `kernel.ops.read`) counts the waiting
  jobs and events of each database and module; a tenant with its own database is its own line.

The repository ships ready-made files:

- [`deploy/prometheus/autoxide-alerts.yml`](../deploy/prometheus/autoxide-alerts.yml): alerting
  rules — server errors above 2%, slow p95, a job waiting more than 10 minutes, jobs going to the
  dead-letter queue, events not delivered for 5 minutes, a connection pool 90% used. Each rule has an
  `action` annotation saying what to do; the steps are in the runbook below. Add it to `rule_files`
  in `prometheus.yml` and route the alerts with Alertmanager.
- [`deploy/grafana/autoxide-dashboard.json`](../deploy/grafana/autoxide-dashboard.json): a Grafana
  dashboard (Dashboards › New › Import) with requests, errors, p95 by route, backlogs, the dead-letter
  queue and connections, filtered by database.

With several worker processes each one publishes the same gauges (one series per `instance`); the
rules and the dashboard take the largest value, not the sum.

## Runbook

Each case: what you see, how to check, how to fix it.

### Server errors

- **Sign:** alert `AutoxideHigh5xxRate`; clients get `500` problems.
- **Check:** every error response carries a `request_id`; search the logs for it. `autoxide doctor`
  checks the configuration, the database, migrations, file storage and mail in one go.
- **Fix:** what the log says (a database that went away, a full disk, an outside service down). An
  error inside the code is a bug: report it with the log lines, without secrets or personal data.

### Slow requests

- **Sign:** alert `AutoxideSlowP95` names the route.
- **Check:** whether `database_connections` is near its maximum (requests wait for a connection), and
  the query plan of that route's queries (`EXPLAIN ANALYZE` on PostgreSQL, `EXPLAIN QUERY PLAN` on
  SQLite).
- **Fix:** a missing index belongs in a new migration of the module that owns the table; a pool too
  small, see "Database connections".

### Stuck queue

- **Sign:** alert `AutoxideJobsStuck` or `AutoxideEventsBacklog`; mail, webhooks or reports arrive late.
- **Check:** that at least one process runs with `AUTOXIDE_PROCESS_ROLE` `all` or `worker`
  (`/health/ready` of that process, its logs); `jobs_pending` by module shows which module is behind;
  in multi-tenant mode a database still being migrated (`503 tenant_maintenance`) waits until
  `autoxide migrate up --all` finishes.
- **Fix:** start (or restart) the worker; raise `AUTOXIDE_JOBS_CONCURRENCY` or add worker processes
  when one module always has a long queue. Events wait for a subscriber that keeps failing: its error
  is in the logs (`event delivery failed`).

### Dead-letter queue

- **Sign:** alert `AutoxideDeadLetterGrowing`; `jobs_dead` goes up.
- **Check:** `autoxide jobs failed` lists the jobs that gave up, newest first, with their last error
  (`--module` to narrow).
- **Fix:** fix the cause first (a wrong SMTP password, an outside service down), then
  `autoxide jobs retry <module> <id>` for each job. A job retried before the fix goes back to the
  queue only to fail again.

### Too many connections

- **Sign:** requests wait before any byte is answered; `/health/ready` is slow or times out, while the
  process is otherwise healthy.
- **Check:** open connections (`ss -tn state established '( sport = :8080 )' | wc -l`) against
  `AUTOXIDE_HTTP_MAX_CONNECTIONS`; at the limit, new connections — health checks on the same port
  included — wait until one closes.
- **Fix:** limit connections per client in the reverse proxy (most of a flood comes from few clients),
  then raise the setting together with the open file limit. Give the liveness probe a timeout and a
  failure threshold that ride out a short saturation (`/health/live`, for example 5 seconds × 3): a
  restart drops every connection; a slow readiness probe only takes the instance out of rotation, which
  is what you want while it is full.

### Database connections

- **Sign:** alert `AutoxideDbPoolExhausted`; slow requests that wait for a connection.
- **Check:** `database_connections{state="in_use"}` by pool; on PostgreSQL, long queries in
  `pg_stat_activity`.
- **Fix:** end what holds connections (a slow query, a stuck job), then raise
  `AUTOXIDE_DATABASE_READ_CONNECTIONS` / `AUTOXIDE_DATABASE_WRITE_CONNECTIONS` — every process opens
  that many, so keep the total under the server's `max_connections`. SQLite has one write connection
  by design.

### Database full

- **Sign:** writes fail with "disk full" or "no space left on device" in the logs.
- **Check:** SQLite: the size of the database file and of its `-wal` file (a long-running read, a
  backup for example, keeps the `-wal` file growing until it ends). PostgreSQL: the largest tables,
  `SELECT relname, pg_size_pretty(pg_total_relation_size(relid)) FROM pg_statio_user_tables ORDER BY
  pg_total_relation_size(relid) DESC LIMIT 10;`.
- **Fix:** free or add disk space first. Finished jobs and delivered events are removed after 7 days
  on their own; on PostgreSQL run `VACUUM` on a large table after deleting much from it.

### Backups

- **Sign:** alert `AutoxideBackupTooOld` or `AutoxideBackupFailed`; `autoxide doctor` warns about
  `kernel.backup_age`.
- **Check:** the newest rows of `kernel_backups` for that database (`status`, `error`), the log line
  `backup failed`, free space on the server (a backup is first built in the temporary directory, as
  large as the database) and on the file storage.
- **Fix:** what the error says (no `files` module, a full disk, storage credentials). A row stuck
  in `running` whose process died is marked `failed` by the next run after 15 minutes. Take a backup
  by hand with `autoxide backup <directory>` until the scheduled ones work again.

### A key leaked

- `AUTOXIDE_AUTH_TOKEN_SECRET`: set a new value on every server and restart, WITHOUT putting the leaked
  one in `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS`. Every access token stops working; clients get a new
  one with their refresh token, which stays valid. (Routine changes keep the old key there for a
  while so nobody notices — see "Get the code and set the secrets".)
- `AUTOXIDE_SECRET_KEY`: rotate it as described in "Get the code and set the secrets" (new value,
  old one in `AUTOXIDE_SECRET_KEY_PREVIOUS`, `autoxide secrets reseal`). Backups taken before still open
  with the old key: delete them or keep them as safe as the key. Counters by visitor address start
  again under the new key.
- An API key or a webhook secret: revoke the key, rotate the webhook secret from the API.

### The last administrator is gone

- **Sign:** nobody can sign in with the `admin` role (the account was removed, locked out, or the
  person left). `autoxide doctor` warns when no active administrator exists.
- **Fix:** on the server, `autoxide user create --email … --name … --admin` makes a new administrator
  and prints its password. For an administrator who only lost their second factor or passkeys:
  `autoxide user reset-mfa` / `autoxide user remove-passkeys`.

### Updating while two versions run

During a rolling update, old and new servers run side by side on the same database, so a release
changes the database in two steps ("expand, then contract"):

1. **Expand:** the new version adds what it needs (a new column, nullable or with a default; a new
   table) and writes both the old and the new shape. The old version keeps working: it does not know
   the new column.
2. **Contract:** a later release, once no server of the old version runs, stops writing the old shape
   and drops it.

`autoxide serve` refuses to start while migrations are missing, so run `autoxide migrate up` (with
`--all` in multi-tenant mode) before starting the new servers, then replace the old ones as in
"Updating without dropping requests".

## Checklist before going live

- [ ] `AUTOXIDE_AUTH_TOKEN_SECRET` and `AUTOXIDE_SECRET_KEY` set to long random values, kept
      secret (`autoxide doctor` warns when one is missing)
- [ ] TLS in front (reverse proxy), port 8080 not reachable from the internet
- [ ] `AUTOXIDE_HTTP_CLIENT_IP_HEADER` set to your proxy's header
- [ ] SMTP configured, so password reset and email verification work
- [ ] Backups taken on a schedule and copied to another machine; a restore tried once
- [ ] `/health/ready` watched by your monitoring
- [ ] Placement countries recorded (`autoxide placement country`) when tenants must keep data in Vietnam,
      and `incident.alert_emails` set for the 72-hour breach reminders
