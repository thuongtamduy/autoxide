# Autoxide

**English** · [Tiếng Việt](README.vi.md)

Autoxide is an open-source backend core written in Rust. The core is built first;
business modules plug in on top of it. It runs on **both SQLite and PostgreSQL**
from a single code base, exposes a REST API described by OpenAPI, and is designed
to be used by AI agents as well as by people.

It is a foundation for new products. It is not tied to any company or industry:
the core contains no business concepts — those belong to modules.

**A commerce-ready backend for the Vietnamese market.** Taxes (a dated tax data pack), accounting
documents and invoices, the official list of administrative units for addresses and the legal rules
(e-commerce, personal data, record keeping) come built in. AI agents use it through MCP and the
OpenAPI contract, with the same permissions as people. The core stays neutral and modules plug in.
Payment, SMS, shipping and e-invoice providers sit behind interfaces that ship today with
simulated providers only: connecting a real provider is still to be done.

## Quickstart

One command, nothing to compile: SQLite in a Docker volume, the tables created, a first
administrator and the reference data loaded. You need
[Docker](https://docs.docker.com/get-docker/) (with Compose) and `openssl`.

```bash
mkdir autoxide && cd autoxide
curl -fsSLO https://raw.githubusercontent.com/thuongtamduy/autoxide/main/compose.quickstart.yml
# Two secrets, kept in .env next to it (never commit it)
printf 'AUTOXIDE_AUTH_TOKEN_SECRET=%s\nAUTOXIDE_SECRET_KEY=%s\n' \
  "$(openssl rand -base64 48 | tr -d '\n')" "$(openssl rand -base64 48 | tr -d '\n')" > .env

docker compose -f compose.quickstart.yml up -d
# The administrator's generated password (see below)
docker compose -f compose.quickstart.yml exec autoxide cat data/initial-admin-password
```

**The first administrator's password.** On its first start the server creates the administrator
`admin@example.com` with a generated password. The password never goes to the log (logs are often
collected elsewhere): it is written once to the file `data/initial-admin-password`, which only the
server's user can read. Inside the container that is `/var/lib/autoxide/data/initial-admin-password`,
in the Docker volume `data` (named `<directory>_data` by Compose), so it survives restarts. The
command above prints its two lines: the email, then the password. The log only says where the file
is (`docker compose -f compose.quickstart.yml logs autoxide | grep "first administrator"`). If `cat`
answers "No such file", an administrator existed already (the volume was used before), or you set
`AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD` yourself. Once you have signed in and changed the password,
delete the file:
`docker compose -f compose.quickstart.yml exec autoxide rm data/initial-admin-password`.

Then open <http://127.0.0.1:8080/> (name, version and links) and sign in. The API reference page
(`/api/docs`, it loads its script from a CDN) is off in the quickstart: add
`AUTOXIDE_HTTP_API_DOCS=true` to `.env` and run `docker compose -f compose.quickstart.yml up -d`
again to turn it on.

```bash
curl -sS http://127.0.0.1:8080/api/v1/auth/login -H 'Content-Type: application/json' \
  -d '{"email":"admin@example.com","password":"<the password from the file>"}'
```

The guides in [`docs/guides/`](docs/guides/README.md) write the same request shorter with
`--json '…'`, which needs curl 7.82 or newer (`curl --version`); with an older curl, write
`-H 'Content-Type: application/json' -d '…'` instead.

To choose them yourself, add `AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL=…` and
`AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD=…` to `.env` before the first start. Next: connect an AI agent
over MCP ([`docs/guides/mcp.md`](docs/guides/mcp.md#6-from-the-quickstart-to-a-working-agent)). The
quickstart needs a release made after 0.16.0; for PostgreSQL, several servers and HTTPS, see
[`docs/DEPLOY.md`](docs/DEPLOY.md).

### What the quickstart contains

The released image contains every module of the `autoxide` binary (all of `modules/` except the
sample `notes`). The server runs the core and official ones by default; the experimental ones
(marked \* below) are off until you name them in `AUTOXIDE_MODULES` — see
[`docs/MODULES.md`](docs/MODULES.md) for the tiers and how to choose.
`GET /api/v1/kernel/modules` lists what runs on your server. Each module answers under
`/api/v1/<module>`.

| Area | Modules |
| --- | --- |
| Accounts and access | `auth` (sign-in, roles, API keys, two-step sign-in, passkeys), `organizations` (tenants, units, stores), `audit`, `idempotency` |
| Platform | `files`, `mail`, `notifications`, `webhooks`, `ai` (off until you configure a provider), `address` (Vietnamese administrative units), `compliance`, `plans` (plans and subscriptions), `reporting`\*, `automations`\* |
| Website | `content` (posts, pages), `media`, `menus`, `forms`, `newsletter`\*, `comments`\*, `analytics`\* |
| Commerce | `catalog`, `inventory`, `promotions`, `shipping`, `orders`, `customers`, `tax`, `ledger` (money), `invoices` |
| Service | `bookings`\*, `support`\*, `conversations`\* |

Nothing has to be turned on to try the core and official modules, and a module whose data you
never write costs nothing. Modules that talk to outside services (an AI provider, payment or
shipping partners, SMTP) send nothing out until you configure them. Which modules each tenant may
use is set by its plan ([`docs/PLANS.md`](docs/PLANS.md)); the set the server runs is chosen with
`AUTOXIDE_MODULES` ([`docs/MODULES.md`](docs/MODULES.md)), and listing it needs the
`kernel.modules.read` permission (the administrator has it).

### Changing a record: `If-Match`

Reading one record returns its version in the `ETag` header. Send it back in `If-Match` when you
change the record, so two people never overwrite each other: a stale version gets `412`, and
endpoints that require the header answer `428` without it.

```bash
# Read the post and keep its ETag header (a quoted number, such as "1763700000000000")
curl -sS -i http://127.0.0.1:8080/api/v1/content/entries/<id> -H 'Authorization: Bearer <access token>'
# Change it with that ETag
curl -sS -X PATCH http://127.0.0.1:8080/api/v1/content/entries/<id> \
  -H 'Authorization: Bearer <access token>' -H 'If-Match: <the ETag>' \
  -H 'Content-Type: application/json' -d '{"title":"A new title"}'
```

[`docs/guides/errors.md`](docs/guides/errors.md) shows each answer (`428`, `412`) as a run
example.

## Status

The core is complete: **every step of the roadmap is done** — project skeleton,
foundation, login with roles and permissions, two-step sign-in, events and audit log,
job queue, i18n and idempotency, email, file storage, MCP endpoint, sample module,
tooling and deployment, passkeys, session management, account deletion and data
export, webhooks. Business modules come next. The core has the Cargo workspace, the quality gate, configuration,
database access, per-module migrations, the module contract, the HTTP server
under `/api/v1` with OpenAPI, health checks, logs, metrics, tracing and the CLI.
The `auth` module stores
users with Argon2id password hashes and signs them in with short-lived access
tokens and rotating refresh tokens (`/api/v1/auth/login`, `/refresh`, `/logout`,
`/me`). Access is controlled by roles: each module declares its permissions, roles
group them (`/api/v1/auth/roles`), and a built-in `admin` role has them all. The
first administrator is created from the command line (`autoxide user create`);
after that, administrators manage accounts through `/api/v1/auth/users`. People can
create their own accounts only when `AUTOXIDE_AUTH_SELF_REGISTRATION=true`.
Programs and AI agents use API keys (`/api/v1/auth/api-keys`): a key acts as the
user who created it, limited to the permissions listed on it, and is sent like an
access token (`Authorization: Bearer axk1_…`; the key names its tenant, so no other header
is needed). Creating a key, like adding a passkey, asks
for the user's current password, and resetting a password removes every key and passkey
of that account. Users can turn on two-step sign-in
with an authenticator app (TOTP) and get one-time recovery codes
(`/api/v1/auth/me/mfa`). Every change is kept in an append-only audit log — who,
what, which record, when, before and after — readable at `/api/v1/audit` with the
`audit.read` permission. Background jobs live in the database too (no Redis): retried
with backoff, kept when they fail for good (`autoxide jobs failed`, `autoxide jobs
retry`), and run on a schedule when a module asks. `AUTOXIDE_PROCESS_ROLE` splits a
deployment into API processes and worker processes. Error messages come in English or
Vietnamese, chosen with the `Accept-Language` header. Write requests sent with an
`Idempotency-Key` header are safe to retry: a repeat returns the first result (except a
result holding a one-time secret, such as a new API key, which is never stored: the repeat
gets `409 idempotency_result_not_kept`). Mail
goes out through SMTP from the job queue (password reset, email verification). Files
are stored on local disk or in S3. AI agents call the system as tools over MCP at
`/mcp`, read its resources (module guides, JSON Schemas) and prompts, and find their way from
`/llms.txt`, a page generated from the code.

What the core contains, in build order (see [`docs/ROADMAP.md`](docs/ROADMAP.md)):

1. Foundation: configuration, database access, migrations per module, the module
   contract, one error type, HTTP under `/api/v1`, OpenAPI, health checks, logs,
   metrics, tracing, CLI
2. Login, roles and permissions, API keys for machines and AI agents
3. MFA (TOTP and recovery codes)
4. Events and an append-only audit log
5. A job queue stored in the database (no Redis)
6. i18n (English, Vietnamese) and idempotency keys
7. Email, password reset, email verification
8. File storage (local disk and S3)
9. An MCP endpoint so AI agents can call the system as tools
10. A sample module and a guide for writing modules
11. Tooling and deployment: data copy between SQLite and PostgreSQL, benchmarks,
    Docker, backup and restore, a one-command check (`./gate.sh`)
12. Passkeys; 13–14. account management, webhooks, a hardened API contract
15. A media library: images, video, audio and documents in nested folders, served
    publicly or privately ([`docs/MEDIA.md`](docs/MEDIA.md))
16. Content: posts and pages in several languages, drafts and history, scheduling,
    categories, tags, authors, menus, redirects, and public endpoints for the website
    ([`docs/CONTENT.md`](docs/CONTENT.md))
17. AI features, off by default, with the provider you choose (Claude, OpenAI, Gemini,
    Ollama…): SEO and alt text suggestions, translation into drafts, search by meaning,
    and permissions that keep AI agents to their own work ([`docs/AI.md`](docs/AI.md))
18. Forms: contact and sign-up forms defined through the API, answers checked on the
    server, spam held back, an inbox with CSV export ([`docs/FORMS.md`](docs/FORMS.md))
19. Newsletters: lists, double opt-in, one-click unsubscribe, campaigns sent in batches
    with pause and resume, bounces, mail for new posts ([`docs/NEWSLETTER.md`](docs/NEWSLETTER.md))
20. Comments: two-level comments on any target for signed-in users or guests, a fixed
    moderation pipeline, reports, bans, reply mails ([`docs/COMMENTS.md`](docs/COMMENTS.md))
21. Website menus: main bar, mega menus and footer whose items link to addresses or to objects of
    any module, with automatic sub-items, dated badges and a cached public endpoint
    ([`docs/MENUS.md`](docs/MENUS.md))
22. Organizations and identity: an organization tree in each tenant (head office, regions,
    stores…) with permissions scoped to a unit or a branch, settings inherited down the tree,
    a website per unit, public store profiles; customer accounts apart from staff, invitations,
    sign-in links, delegation down chains of tenants, time-limited support access; the official
    list of Vietnamese administrative units for addresses ([`docs/ADDRESS.md`](docs/ADDRESS.md))

## Try it with Docker (no build)

The same as the [Quickstart](#quickstart), step by step with plain `docker run` (works with every
release): nothing to compile, SQLite inside a Docker volume. You need [Docker](https://docs.docker.com/get-docker/) and `openssl`. The
image is about 410 MB on disk and the server is ready about a second after it starts.

```bash
# Two secrets, made up here and kept in a file (use the same ones on every start; never commit it)
printf 'AUTOXIDE_AUTH_TOKEN_SECRET=%s\nAUTOXIDE_SECRET_KEY=%s\n' \
  "$(openssl rand -base64 48 | tr -d '\n')" "$(openssl rand -base64 48 | tr -d '\n')" > autoxide.env
chmod 600 autoxide.env

IMAGE=ghcr.io/thuongtamduy/autoxide:0.17.0   # a release from the GitHub Releases page
RUN="docker run --rm --env-file autoxide.env -v autoxide-data:/var/lib/autoxide/data"

# 1. Create the tables (the server will not start before this, and again after every update)
$RUN $IMAGE migrate up
# 2. The first administrator: a strong password is printed once
$RUN $IMAGE user create --email you@example.com --name "Your Name" --admin
# 3. Reference data (addresses, taxes, promotion limits); each is safe to run again
$RUN $IMAGE task address.import
$RUN $IMAGE task tax.import_pack
$RUN $IMAGE task promotions.load_legal_data
# 4. Start the server (it listens inside the container; this publishes it on this machine only)
docker run -d --name autoxide --restart unless-stopped --env-file autoxide.env \
  -v autoxide-data:/var/lib/autoxide/data -p 127.0.0.1:8080:8080 $IMAGE serve

curl http://127.0.0.1:8080/health/ready     # {"data":{"status":"ok"}}
$RUN $IMAGE doctor                           # what is still missing before going live
```

Keep `-v autoxide-data:/var/lib/autoxide/data` on every command: the database, uploaded files
and mail live in that volume, and without it they vanish with the container. A backup must go
to a directory you mount, not to a path inside the container (`mkdir backups && sudo chown 10001
backups`, then add `-v "$PWD/backups:/backups"` and run `backup /backups/<name>`). Sign in as
shown under [Configuration](#configuration) (use `http://127.0.0.1:8080`). For PostgreSQL,
a reverse proxy with HTTPS, updates and restore, see [`docs/DEPLOY.md`](docs/DEPLOY.md).

## Requirements

These are only needed to build from source (to run a released image, see above).

- [Rust](https://rustup.rs) — `rustup` installs the exact toolchain pinned in
  `rust-toolchain.toml` the first time you run `cargo`
- A C compiler (SQLite and libwebp are compiled from source and bundled into the binary;
  on Debian or Ubuntu: `sudo apt install build-essential`)
- To run the full quality gate: [Docker](https://docs.docker.com/get-docker/) and
  [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny)
  (`cargo install cargo-deny --locked`)

**Building takes a while.** The first build (`cargo build --release`, or `docker build`) compiles
several hundred crates: count on roughly 20 minutes to well over an hour, depending on the number
of CPU cores and the memory (at least 4 GB of RAM, 8 GB is comfortable; raise Docker Desktop's
memory limit if a build is killed). Later builds reuse what was compiled. To run Autoxide rather
than change it, use the released image above: nothing to compile.

**Behind a company proxy.** `cargo` and `rustup` read `HTTPS_PROXY` (and `HTTP_PROXY`, `NO_PROXY`)
from the environment. `docker build` does not pass your shell's variables into the build: give them
as build arguments, which Docker knows already and does not keep in the image —

```bash
docker build --build-arg HTTPS_PROXY=http://proxy.example.com:3128   --build-arg HTTP_PROXY=http://proxy.example.com:3128 --build-arg NO_PROXY=localhost,127.0.0.1   -t autoxide .
```

— or set them once under `proxies` in `~/.docker/config.json`, which Docker passes to every build,
`docker compose build` included. A proxy that inspects HTTPS with its own certificate makes the
downloads fail with certificate errors inside the build; use the released image then.

## Getting started

```bash
git clone https://github.com/thuongtamduy/autoxide.git autoxide
cd autoxide

# Build and run the command-line interface
cargo run -p autoxide -- --help

# Run the tests on SQLite (no setup needed)
cargo test --workspace

# Run the full quality gate: format, lints, boundary checks, dependency audit,
# tests on SQLite, tests on PostgreSQL (started in a throwaway Docker container)
./gate.sh
```

If you are going to commit, enable the Git hooks once. They scan staged changes
for secrets (when [`gitleaks`](https://github.com/gitleaks/gitleaks) is installed)
and check formatting:

```bash
git config core.hooksPath .githooks
```

## Configuration

Every setting has a working default, so a fresh clone runs without any
configuration. Settings are read from environment variables and from a `.env`
file in the current directory; environment variables win.

```bash
# Check the configuration and show every setting with its current value
cargo run -p autoxide -- config

# Start your own configuration from the documented defaults
cp .env.example .env
```

The database schema is managed by migrations, one set per module. They are never
applied automatically — run them once per deployment:

```bash
cargo run -p autoxide -- migrate status          # what is applied, what is pending
cargo run -p autoxide -- migrate up              # apply everything that is pending
cargo run -p autoxide -- migrate down <module>   # revert the newest migration of one module
```

Create the first administrator. A strong password is generated and printed once;
to choose your own, pipe it in and add `--password-stdin`:

```bash
cargo run -p autoxide -- user create --email you@example.com --name "Your Name" --admin
```

Import the reference data the server ships with. No step changes your own
data, and running one again changes nothing:

```bash
# The Vietnamese administrative units (provinces, communes), used to check addresses.
# Nothing imports it for you: run it once per installation, and again when an update
# ships a newer list (`doctor` says so). See docs/ADDRESS.md.
cargo run -p autoxide -- task address.import

# The tax data pack (rates, withholding rules). A running server imports it by itself
# within an hour; run this to have it at once. See docs/TAX.md.
cargo run -p autoxide -- task tax.import_pack

# The legal limits on promotions (Vietnam). Without them promotions cannot be turned on
# (`doctor` warns). See docs/PROMOTIONS.md.
cargo run -p autoxide -- task promotions.load_legal_data
```

Check the installation. `doctor` goes through the configuration, the database,
the migrations, the signing keys, file storage, the SMTP server, the administrator
account and the imported reference data, and says how to fix each problem (`--json` for scripts and AI agents; it exits with
status 1 when a check failed):

```bash
cargo run -p autoxide -- doctor
```

Modules can add maintenance tasks; `autoxide task` lists them with their arguments
(`cargo run -p autoxide -- task`).

Then start the server. It refuses to start while a migration is pending, and
stops gracefully on Ctrl-C or `SIGTERM` (requests in progress get
`AUTOXIDE_HTTP_SHUTDOWN_SECONDS` to finish; behind a load balancer, see
[updating without dropping requests](docs/DEPLOY.md#updating-without-dropping-requests)):

```bash
cargo run -p autoxide -- serve
```

By default it listens on `127.0.0.1:8080`: check it with
`curl http://127.0.0.1:8080/health/ready`. Logs go to standard error. Sign in and
look at your account:

```bash
curl -s http://127.0.0.1:8080/api/v1/auth/login -H 'Content-Type: application/json' \
  -d '{"email": "you@example.com", "password": "<the password>"}'
curl -s http://127.0.0.1:8080/api/v1/auth/me -H 'Authorization: Bearer <access_token>'
```

The API contract is an OpenAPI document generated from the code. It is committed
as [`docs/openapi.json`](docs/openapi.json), printed by `autoxide openapi`, and
served by the running server at `/api/v1/openapi.json`. Its description lists the
conventions every endpoint follows (response envelope, RFC 9457 errors with a
stable `code`, cursor pagination, request ids).

AI agents reach the same functions as tools through the
[Model Context Protocol](https://modelcontextprotocol.io) at `/mcp` (Streamable
HTTP), authenticated like the API — an API key is the usual choice. An agent sees and
calls only the tools its permissions allow; each tool runs the same code as the
matching REST endpoint, so writes land in the audit log. For example, with Claude Code:

```bash
claude mcp add --transport http autoxide http://127.0.0.1:8080/mcp --header "Authorization: Bearer <api key>"
```

[`docs/guides/mcp.md`](docs/guides/mcp.md) walks through it: a key with the least
permissions, other MCP clients, every tool with the permission it needs, and the limits.

Back up everything — the data of every module and every stored file — into a
directory, and restore it into a new, empty database (with any file storage) later. The
database is read in one consistent snapshot, so the server may keep running:

```bash
cargo run -p autoxide -- backup backups/2026-11-11
cargo run -p autoxide -- restore backups/2026-11-11
```

Restore only backups you made yourself: a backup is the whole content of the server.

To move to PostgreSQL later (or bring data back to SQLite), stop the server and copy
everything into a new, empty database; it gets every migration first:

```bash
printf '%s' 'postgres://user:password@host:5432/autoxide' | cargo run -p autoxide -- copy --to-stdin
```

[`.env.example`](.env.example) lists every setting with its description and
default. It is generated from the code, so it cannot go out of date. An invalid
configuration is reported in full — every problem at once, each saying what to
fix — and the command exits with status 2. Secrets are never printed.

Each client address may make 600 requests a minute by default
(`AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE`), and an email is locked for 15 minutes after
5 failed sign-ins. Behind a reverse proxy, set `AUTOXIDE_HTTP_CLIENT_IP_HEADER` (for
example to `X-Forwarded-For`) so that clients are told apart by their own address
rather than the proxy's.

Users can also sign in with a passkey (Face ID, fingerprint, security key) instead of a
password. Set `AUTOXIDE_AUTH_PASSKEY_RP_ID` to your site's domain (and
`AUTOXIDE_AUTH_PASSKEY_ORIGINS` when your web app runs on a subdomain); a signed-in user
adds one with `POST /api/v1/auth/me/passkeys/registration` and `POST /api/v1/auth/me/passkeys`,
and signs in with `POST /api/v1/auth/login/passkey/options` and
`POST /api/v1/auth/login/passkey` — no email needed. The `options` returned go straight to
`PublicKeyCredential.parseCreationOptionsFromJSON()` / `parseRequestOptionsFromJSON()` in the
browser.

Signing in with Google, Microsoft, GitHub or any OpenID Connect provider is off by default;
turn it on with `AUTOXIDE_AUTH_OIDC=true`, then each tenant adds its providers with
`POST /api/v1/auth/oidc-providers` (permission `auth.oidc.manage`). An existing account is linked
only when both sides have verified the email, and two-step sign-in still applies. See
[`docs/guides/oidc.md`](docs/guides/oidc.md).

Users with the `files.upload` permission upload a file by sending its raw content to
`POST /api/v1/files?name=report.pdf` with its `Content-Type`, then list, download
(`GET /api/v1/files/{id}/content`) and delete their own files; `files.manage` reaches
everyone's. Uploads are limited to 25 MiB (`AUTOXIDE_FILES_MAX_BYTES`) and, when you
set `AUTOXIDE_FILES_ALLOWED_TYPES` (for example `image/*,application/pdf`), to those
media types.

Uploaded files are stored in `data/files` until you set `AUTOXIDE_FILES_S3_BUCKET`
(with `AWS_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_ENDPOINT_URL`
for an S3-compatible service such as Cloudflare R2). Back up `data/files` together with
the database.

Mail is written as `.eml` files to `data/mail` until you set `AUTOXIDE_MAIL_SMTP_URL`
(for example `smtps://user:password@smtp.example.com:465`) and `AUTOXIDE_MAIL_FROM`.

A user who forgot their password calls `POST /api/v1/auth/password-reset` with their
email and receives a one-time token by mail, valid for 30 minutes; they then call
`POST /api/v1/auth/password-reset/confirm` with the token and a new password. The
answer is the same whether or not the email has an account. Set
`AUTOXIDE_AUTH_PASSWORD_RESET_URL` (for example
`https://app.example.com/reset?token={token}`) to send a link to your own page
instead of the bare token.

Signed-in users change their own password with `POST /api/v1/auth/me/password`
(the current password is required). An administrator can give the first password to
an account created without one (`PUT /api/v1/auth/users/{id}/password`), but cannot
replace a password someone already has.

Each sign-in is a session, named by the `session_id` in the token response. Users see
their sessions — device and last activity — at `GET /api/v1/auth/me/sessions`, sign one
out with `DELETE /api/v1/auth/me/sessions/{id}`, or every other one with
`DELETE /api/v1/auth/me/sessions?keep={their session_id}`. Users download everything the
server keeps about them with `GET /api/v1/auth/me/export` and delete their account with
`DELETE /api/v1/auth/me` (current password required); every module then deletes its data
about them. Administrators find users with `GET /api/v1/auth/users?search=ada`, sign a
user out everywhere (`DELETE /api/v1/auth/users/{id}/sessions`) and delete accounts
(`DELETE /api/v1/auth/users/{id}`). An administrator cannot act this way on someone who
holds permissions they lack.

Other systems receive events through webhooks: with the `webhooks.manage` permission,
register an `https://` address and the events it wants at `POST /api/v1/webhooks` (the
list of events is at `GET /api/v1/webhooks/events`). Each event is sent as a signed
`POST` following [Standard Webhooks](https://www.standardwebhooks.com), so the receiver
can check it with an existing library and the secret returned at creation. Failed
deliveries are retried for about two hours; `GET /api/v1/webhooks/{id}/deliveries` shows
what was sent and what failed. Webhooks go only to public addresses: an address in a
private or local network (`localhost`, `10.x`, `192.168.x`, `169.254.x`…) is refused,
unless `AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS=true` because the receivers run in your
own network.

New accounts receive a mail asking to confirm their address; the token in it (valid
24 hours) is confirmed with `POST /api/v1/auth/email-verification/confirm`, and
`email_verified_at` then shows when. To change their email, users call
`POST /api/v1/auth/me/email` with the new address and their password: the email
changes only once the new address is confirmed the same way. The old address then gets a
mail saying so, with a token (valid 7 days) to report a change the owner did not make:
`POST /api/v1/auth/email-change/report` signs the account out everywhere, removes its
password, passkeys, API keys and two-step sign-in, gives the old address back when it is
still free, and sends a password-reset mail there. Set
`AUTOXIDE_AUTH_EMAIL_VERIFICATION_URL` and `AUTOXIDE_AUTH_EMAIL_CHANGE_REPORT_URL` to link
these mails to your own pages.

For production, set `AUTOXIDE_AUTH_TOKEN_SECRET` (the key that signs access tokens)
to a long random value, the same on every server:

```bash
openssl rand -base64 48
```

Without it the server makes up a key at startup and logs a warning: fine for trying
Autoxide out, but signed-in clients must refresh their access token after every
restart.

### Running the tests against your own PostgreSQL

Tests pick their database from `TEST_DATABASE_URL`. Unset means SQLite. To use
PostgreSQL 16 or newer, point it at any database on a server where the user may
`CREATE DATABASE` — every test creates its own empty database and drops it
afterwards:

```bash
TEST_DATABASE_URL=postgres://user:password@localhost:5432/postgres cargo test --workspace
```

`./gate.sh` does this for you with Docker. To make the gate use an existing
server instead, set `GATE_POSTGRES_URL` to the same kind of URL.

Tests that build the whole application copy a template database that already has
every migration applied, instead of migrating each empty database again. The
templates live in the system temporary directory (`autoxide-templates/`) and, on
PostgreSQL, as databases named `autoxide_tpl_…`; both are rebuilt when the test
program or its migrations change, and removed after an hour. Set
`TEST_DATABASE_TEMPLATE=off` to migrate every test database from scratch.

File storage tests also run against S3 when `TEST_S3_ENDPOINT` points at an
S3-compatible service with a bucket named `autoxide-test`. The gate starts a throwaway
S3 emulator ([moto](https://github.com/getmoto/moto)) in Docker for this; set
`GATE_S3_ENDPOINT` to use an existing service instead.

## Project layout

| Path             | What it is                                                              |
| ---------------- | ----------------------------------------------------------------------- |
| `crates/kernel`  | The foundation every module builds on. Knows no module.                 |
| `crates/app`     | The `autoxide` binary: the one place where modules are registered.      |
| `crates/testkit` | Test support: an isolated database per test, on SQLite or PostgreSQL.   |
| `modules/<name>` | One crate per module, each owning its tables (`auth` accounts and sign-in, `audit`, `files`, `mail`, `webhooks`, `content`, `media`, `catalog`, `orders`… — about thirty; `ls modules` lists them, the sections above say what each does). |
| `xtask`          | Developer tooling: `cargo xtask check` enforces the workspace boundaries. |
| `gate.sh`        | The quality gate. A change is done only when this is green.             |

Modules own their tables, talk to each other only through published traits, and
never share foreign keys. These rules are checked automatically by the gate.

## Documentation

- [`STACK.md`](STACK.md) — which libraries are used and why
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — what is built, in what order
- [`docs/decisions/INDEX.md`](docs/decisions/INDEX.md) — the decision log by topic (the old
  entries are in [`docs/DECISIONS.md`](docs/DECISIONS.md), new ones one file each in `docs/decisions/`)
- [`CLAUDE.md`](CLAUDE.md) — the rules for contributors, human or AI agent
- [`CHANGELOG.md`](CHANGELOG.md) — what changed in each release; released Docker images
  are at `ghcr.io/<owner>/autoxide`
- [`docs/guides/`](docs/guides/README.md) — task-by-task guides to the API, with `curl` and
  TypeScript examples that run as tests: a website, signing in, API keys, retries,
  pagination, errors, connecting an AI agent over MCP
- [`sdk/typescript/`](sdk/typescript/README.md) — a typed TypeScript client generated from
  `docs/openapi.json`; [`docs/mcp-tools.json`](docs/mcp-tools.json) — the contract of every MCP tool
- [`docs/DEPLOY.md`](docs/DEPLOY.md) — installing and deploying: Docker Compose with
  PostgreSQL or one binary with SQLite, reverse proxy, updates, backups
- [`docs/MEDIA.md`](docs/MEDIA.md) — using the media library: upload, folders, public
  links, caching, trash
- [`docs/CONTENT.md`](docs/CONTENT.md) — using the content module: drafts, publishing,
  pages and addresses, languages, public endpoints for the website, preview
- [`docs/AI.md`](docs/AI.md) — the AI features: turning them on with each provider,
  embeddings, costs and limits, what is sent, limits for AI agents
- [`docs/FORMS.md`](docs/FORMS.md) — using the forms module: defining a form, showing it
  on a website, anti-spam, the inbox, data requests by email
- [`docs/NEWSLETTER.md`](docs/NEWSLETTER.md) — using the newsletter module: lists, the
  website's sign-up, confirmation and preferences pages, campaigns, bounces, legal notes
- [`docs/COMMENTS.md`](docs/COMMENTS.md) — using the comments module: targets, the
  website's thread and comment box, moderation, reports and bans, reply mails, legal notes
- [`docs/MENUS.md`](docs/MENUS.md) — using the menus module: items, objects of other modules,
  badges, the public endpoint and its caching, moving from the content module's menus
- [`docs/BOOKINGS.md`](docs/BOOKINGS.md) — bookings: services, staff and rooms, weekly schedules in
  the unit's time zone, free times, deposits through the payment partner, cancellation policy,
  reminders, guests with a phone code
- [`docs/CUSTOMERS.md`](docs/CUSTOMERS.md) — customers: profiles by account or phone number, purchase
  history from orders, masked contact details with audited reveal, manual and rule groups
- [`docs/SUPPORT.md`](docs/SUPPORT.md) — customer support tickets: guests with a ticket token, staff
  scope by unit, teams and routing, business-hour deadlines, hand-over to consumer complaints, AI drafts
- [`docs/ANALYTICS.md`](docs/ANALYTICS.md) — cookieless website analytics: sending events from a
  website, reports, funnels, consent for per-person tracking, legal notes
- [`docs/ADDRESS.md`](docs/ADDRESS.md) — administrative units and addresses: reading the list,
  importing the official list
- [`docs/CATALOG.md`](docs/CATALOG.md) — products: variants, categories, price lists with history,
  price floors
- [`docs/PROMOTIONS.md`](docs/PROMOTIONS.md) — promotions: automatic discounts, coupons, vouchers,
  stacking, usage holds for orders, legal limits as dated data
- [`docs/TAX.md`](docs/TAX.md) — taxes: the Vietnam tax data pack, computing taxes, tax profiles
- [`docs/MONEY.md`](docs/MONEY.md) — money: the ledger, payments and flow modes, VietQR, accounting
  documents, exchange rates, printing, accounting exports, keeping records 10 years
- [`docs/NOTIFICATIONS.md`](docs/NOTIFICATIONS.md) — notifications: kinds and categories, channels,
  consent and allowed hours for advertising, inbox and preferences, declaring a kind in a module
- [`docs/MODULE_GUIDE.md`](docs/MODULE_GUIDE.md) — how to write a module, starting
  from the sample module [`modules/notes`](modules/notes) (not registered by default)

The first four documents steer the project and are written in Vietnamese, with
technical terms kept in English. User-facing documentation (this README, the
installation and usage guides, API descriptions) is written in English with a
Vietnamese translation alongside.

## Contributing

While you work, `./gate.sh --quick` checks a change to one module in under a minute: it runs the tests of
the crates you changed (and of the crates that depend on them) on SQLite only. Before a
change is merged, run the full `./gate.sh`; a change is accepted only when the full gate is
green on both SQLite and PostgreSQL. Never commit secrets — only `.env.example`
belongs in Git, never `.env`. [`CONTRIBUTING.md`](CONTRIBUTING.md) explains how to set up
your machine, the rules that matter most and how to send a change.

## Security

Found a vulnerability? Please do not open a public issue: report it privately as
described in [`SECURITY.md`](SECURITY.md), which also lists the versions that receive
security fixes.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
