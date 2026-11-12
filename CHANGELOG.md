# Changelog

Notable changes to Autoxide. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html): while the version is `0.x`, a
minor release (`0.2.0`) may contain breaking changes, which are always listed under
**Changed** or **Removed**. The HTTP API itself never breaks within `/api/v1`: replaced
endpoints are marked deprecated with a sunset date first.

## [Unreleased]

### Added

- Session management: `GET /api/v1/auth/me/sessions` lists your signed-in devices,
  `DELETE /api/v1/auth/me/sessions/{id}` signs one out, `DELETE /api/v1/auth/me/sessions`
  signs out all (or all but `keep`); administrators use
  `DELETE /api/v1/auth/users/{id}/sessions`. Token responses carry `session_id`.
- Account deletion (`DELETE /api/v1/auth/me`, `DELETE /api/v1/auth/users/{id}`) and personal
  data export (`GET /api/v1/auth/me/export`) across every module. Modules join through
  `autoxide_kernel::privacy` (`PersonalData`, the `auth.user_deleted` event).
- User search: `GET /api/v1/auth/users?search=…` (and `search` in the `list_users` MCP
  tool) finds users by email or name, in any case.
- MCP tools for roles, API keys, the audit log and files (content as base64, up to 1 MiB).
- Webhooks (`/api/v1/webhooks`, permission `webhooks.manage`): events are sent to your
  `https://` endpoints, signed following Standard Webhooks, retried for about two hours,
  with a delivery log. New settings `AUTOXIDE_WEBHOOKS_ALLOW_HTTP` and
  `AUTOXIDE_WEBHOOKS_TIMEOUT_SECONDS`.

### Changed

- Validation reports every problem of a request at once (for example a bad email and an
  empty name together). Webhook endpoints report a non-`https` address or an unknown event
  as field errors (`url`, `events[i]`) like roles and API keys do.
- Creating a record answers `201` with a `Location` header. The OpenAPI document declares
  `Accept-Language` and `Idempotency-Key` on each endpoint and has a `servers` entry.
- MCP tool results have the same shape as REST: lists are `{"data": […], "pagination": …}`
  instead of being wrapped in a second `data`.

- Creating an API key (`POST /api/v1/auth/api-keys`) and starting a passkey registration
  (`POST /api/v1/auth/me/passkeys/registration`) now need `current_password` in the body.
- Resetting a password by email also removes the account's passkeys and API keys, so
  whoever had taken over the account loses every way back in.
- Idempotency keys are scoped to the caller: a request that is not signed in is never
  stored, and two users using the same key do not see each other's answers.

### Fixed

- `docs/openapi.json` (and `/api/v1/openapi.json`) is valid again: the `order` parameter of
  list endpoints referred to a schema missing from the document, so validators and SDK
  generators rejected it. The quality gate now checks every `$ref`.
- MCP tool descriptions are in English only.

### Security

- Concurrent wrong passwords or codes can no longer go past the login throttle; wrong
  codes when turning off two-step sign-in or replacing recovery codes are counted too.
- A refresh racing with disabling the account can no longer issue a new token.
- `auth.users.manage` no longer lets someone reset two-step sign-in, disable, delete, sign
  out or set the first password of a user who has permissions they lack (code
  `auth.target_has_more_permissions`), through REST or MCP.
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
