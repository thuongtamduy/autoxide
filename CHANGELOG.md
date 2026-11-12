# Changelog

Notable changes to Autoxide. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html): while the version is `0.x`, a
minor release (`0.2.0`) may contain breaking changes, which are always listed under
**Changed** or **Removed**. The HTTP API itself never breaks within `/api/v1`: replaced
endpoints are marked deprecated with a sunset date first.

## [Unreleased]

### Changed

- Creating an API key (`POST /api/v1/auth/api-keys`) and starting a passkey registration
  (`POST /api/v1/auth/me/passkeys/registration`) now need `current_password` in the body.
- Resetting a password by email also removes the account's passkeys and API keys, so
  whoever had taken over the account loses every way back in.
- Idempotency keys are scoped to the caller: a request that is not signed in is never
  stored, and two users using the same key do not see each other's answers.

### Security

- Concurrent wrong passwords or codes can no longer go past the login throttle; wrong
  codes when turning off two-step sign-in or replacing recovery codes are counted too.
- A refresh racing with disabling the account can no longer issue a new token.
- `auth.users.manage` no longer lets someone reset two-step sign-in, disable or set the
  first password of a user who has permissions they lack (code
  `auth.target_has_more_permissions`).
- Finished jobs drop their payload, so links with tokens do not stay in the database.
- The rate limiter reads only the last value of the client IP header, as added by your
  proxy; `autoxide restore` refuses backups that contain symbolic links; idempotency waits
  for a request body no longer than the request timeout.

## [0.1.0] - 2026-11-11

First release: the complete core.

### Added

- Foundation: configuration from the environment or `.env` with full validation,
  SQLite and PostgreSQL with per-module migrations, one error type, HTTP API under
  `/api/v1` with an OpenAPI document (`/api/v1/openapi.json`), health checks, structured
  logs, Prometheus metrics, OpenTelemetry tracing, a command-line interface.
- Accounts: sign-in with short-lived access tokens and rotating refresh tokens, roles
  and permissions, API keys for programs and AI agents, two-step sign-in (TOTP and
  recovery codes), passkeys (WebAuthn), password reset and change, email verification
  and email change, login throttling, rate limiting.
- Events and an append-only audit log; a job queue stored in the database with retries,
  schedules and a dead-letter queue.
- Error messages in English and Vietnamese; idempotency keys for safe retries.
- Mail over SMTP (or files during development); file storage on local disk or S3 with
  size and type limits.
- An MCP endpoint (`/mcp`) so AI agents can call the system as tools, with the caller's
  permissions.
- A sample module (`notes`) and a guide for writing modules.
- Tooling: `autoxide copy` between SQLite and PostgreSQL, `autoxide backup` and
  `autoxide restore`, benchmarks and a load test, Docker image and Compose file, CI.

[Unreleased]: https://github.com/thuongtamduy/autoxide/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/thuongtamduy/autoxide/releases/tag/v0.1.0
