# Autoxide

**English** · [Tiếng Việt](README.vi.md)

Autoxide is an open-source backend core written in Rust. The core is built first;
business modules plug in on top of it. It runs on **both SQLite and PostgreSQL**
from a single code base, exposes a REST API described by OpenAPI, and is designed
to be used by AI agents as well as by people.

It is a foundation for new products. It is not tied to any company or industry:
the core contains no business concepts — those belong to modules.

## Status

Early development. **Steps 0 and 1 of the roadmap (project skeleton and
foundation) are done**: the Cargo workspace, the quality gate, configuration,
database access, per-module migrations, the module contract, the HTTP server
under `/api/v1` with OpenAPI, health checks, logs, metrics, tracing and the CLI.
Step 2 (login, roles and permissions) is in progress: the `auth` module stores
users with Argon2id password hashes and signs them in with short-lived access
tokens and rotating refresh tokens (`/api/v1/auth/login`, `/refresh`, `/logout`,
`/me`). There is no way to create the first user yet; that comes with the user
commands of step 2.

What the core will contain, in build order (see [`docs/ROADMAP.md`](docs/ROADMAP.md)):

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
    Docker, backup and restore, CI

## Requirements

- [Rust](https://rustup.rs) — `rustup` installs the exact toolchain pinned in
  `rust-toolchain.toml` the first time you run `cargo`
- A C compiler (SQLite is compiled from source and bundled into the binary)
- To run the full quality gate: [Docker](https://docs.docker.com/get-docker/) and
  [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny)
  (`cargo install cargo-deny --locked`)

## Getting started

```bash
git clone <repository-url> autoxide
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

Then start the server. It refuses to start while a migration is pending, and
stops gracefully on Ctrl-C or `SIGTERM` (requests in progress get
`AUTOXIDE_HTTP_SHUTDOWN_SECONDS` to finish):

```bash
cargo run -p autoxide -- serve
```

By default it listens on `127.0.0.1:8080`: check it with
`curl http://127.0.0.1:8080/health/ready`. Logs go to standard error.

The API contract is an OpenAPI document generated from the code. It is committed
as [`docs/openapi.json`](docs/openapi.json), printed by `autoxide openapi`, and
served by the running server at `/api/v1/openapi.json`. Its description lists the
conventions every endpoint follows (response envelope, RFC 9457 errors with a
stable `code`, cursor pagination, request ids).

[`.env.example`](.env.example) lists every setting with its description and
default. It is generated from the code, so it cannot go out of date. An invalid
configuration is reported in full — every problem at once, each saying what to
fix — and the command exits with status 2. Secrets are never printed.

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

## Project layout

| Path             | What it is                                                              |
| ---------------- | ----------------------------------------------------------------------- |
| `crates/kernel`  | The foundation every module builds on. Knows no module.                 |
| `crates/app`     | The `autoxide` binary: the one place where modules are registered.      |
| `crates/testkit` | Test support: an isolated database per test, on SQLite or PostgreSQL.   |
| `modules/<name>` | One crate per module. `modules/auth`: users and passwords (login, roles and API keys come next). |
| `xtask`          | Developer tooling: `cargo xtask check` enforces the workspace boundaries. |
| `gate.sh`        | The quality gate. A change is done only when this is green.             |

Modules own their tables, talk to each other only through published traits, and
never share foreign keys. These rules are checked automatically by the gate.

## Documentation

- [`STACK.md`](STACK.md) — which libraries are used and why
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — what is built, in what order
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — the decision log
- [`CLAUDE.md`](CLAUDE.md) — the rules for contributors, human or AI agent

These four documents steer the project and are written in Vietnamese, with
technical terms kept in English. User-facing documentation (this README, the
installation and usage guides, API descriptions) is written in English with a
Vietnamese translation alongside.

## Contributing

Run `./gate.sh` before you commit; a change is accepted only when the gate is
green on both SQLite and PostgreSQL. Never commit secrets — only `.env.example`
belongs in Git, never `.env`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
