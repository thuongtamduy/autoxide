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

Create a file named `.env` next to `compose.yml` with two secrets. Never commit it.

```bash
printf 'POSTGRES_PASSWORD=%s\nAUTOXIDE_AUTH_TOKEN_SECRET=%s\n' \
  "$(openssl rand -hex 24)" "$(openssl rand -base64 48 | tr -d '\n')" > .env
chmod 600 .env
```

- `POSTGRES_PASSWORD` protects the database.
- `AUTOXIDE_AUTH_TOKEN_SECRET` signs access tokens. Keep it stable: changing it signs
  everyone out.

### 2. Start

```bash
docker compose up -d --build
```

Or skip building and use a released image (see the project's GitHub releases):

```bash
echo 'AUTOXIDE_IMAGE=ghcr.io/<owner>/autoxide:0.1' >> .env
docker compose up -d --no-build
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
`rust-toolchain.toml`), then:

```bash
cargo build --release --locked -p autoxide
sudo install -m 755 target/release/autoxide /usr/local/bin/autoxide
```

### 2. Prepare a directory

```bash
sudo useradd --system --home-dir /var/lib/autoxide --create-home autoxide
sudo -u autoxide sh -c 'cd /var/lib/autoxide && printf "AUTOXIDE_AUTH_TOKEN_SECRET=%s\n" "$(openssl rand -base64 48 | tr -d "\n")" > .env && chmod 600 .env'
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
| `AUTOXIDE_SECRET_KEY` | Signs short-lived tokens of every module and keys the hashes of visitors' IP addresses; optional: when unset, one is generated once and kept in the database. Set it (outside the database, the same on every server) in production: with the key in the database, anyone holding a database backup can work the IP blocks back out of the stored hashes |
| `AUTOXIDE_HTTP_CLIENT_IP_HEADER` | Behind a reverse proxy: the header with the client address |
| `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` | How many of your proxies a request passes (default `1`) |
| `AUTOXIDE_HTTP_TRUSTED_PROXIES` | Instead of a count: the addresses and ranges of your proxies, e.g. `10.0.0.0/8, 192.0.2.7` |
| `AUTOXIDE_HTTP_PUBLIC_URL` | The address the internet reaches the API at, e.g. `https://api.example.com`; needed for links back to the API in mail (newsletter unsubscribe) |
| `AUTOXIDE_HTTP_CORS_ORIGINS` | Web pages on other origins allowed to call the API |
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

- [ ] `AUTOXIDE_AUTH_TOKEN_SECRET` set to a long random value, kept secret
- [ ] TLS in front (reverse proxy), port 8080 not reachable from the internet
- [ ] `AUTOXIDE_HTTP_CLIENT_IP_HEADER` set to your proxy's header
- [ ] SMTP configured, so password reset and email verification work
- [ ] Backups taken on a schedule and copied to another machine; a restore tried once
- [ ] `/health/ready` watched by your monitoring
