# Changelog

Notable changes to Autoxide. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html): while the version is `0.x`, a
minor release (`0.2.0`) may contain breaking changes, which are always listed under
**Changed** or **Removed**. The HTTP API itself never breaks within `/api/v1`: replaced
endpoints are marked deprecated with a sunset date first.

## [Unreleased]

### Added

- AI helpers (guide in `docs/AI.md`): an SEO check without AI
  (`GET /api/v1/content/entries/{id}/seo-check`); SEO suggestions
  (`POST …/seo-suggestions`) and alt text suggestions (`POST /api/v1/media/{id}/alt-suggestions`,
  from a small copy; private images only with `AUTOXIDE_MEDIA_AI_SEND_PRIVATE=true`) that save
  nothing; translation into the draft of a translation (`POST …/translate` → `202`, a
  background job that keeps structure, links and images, never overwrites a translation
  changed meanwhile and never publishes; `GET /api/v1/content/translation-requests/{id}`,
  events `content.translation_*`, `translation_outdated`); search by meaning combined with
  word search (`GET /api/v1/content/search`, `GET /api/v1/content/public/search` off by
  default), same results on SQLite and PostgreSQL. MCP tools `content.seo_check`,
  `content.suggest_seo`, `content.translate`, `content.get_translation_request`,
  `content.search`, `media.suggest_alt`. New settings `AUTOXIDE_MEDIA_AI_SEND_PRIVATE`,
  `AUTOXIDE_CONTENT_TRANSLATE_BATCH_TOKENS`, `AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH`,
  `AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE`.

- AI connection (module `ai`, off by default): one interface for every module to generate
  structured answers and embeddings, with two adapters — `anthropic` (Claude, Messages API)
  and `openai` (the Chat Completions format: OpenAI, Google Gemini, Ollama, vLLM, LM Studio,
  OpenRouter…) — plus an `/embeddings` client. Every answer follows a JSON Schema and is
  checked again by the server; a daily token limit is kept in the database
  (`AUTOXIDE_AI_DAILY_TOKEN_LIMIT`). `GET /api/v1/ai/status` and the MCP tool `ai.status`
  (permission `ai.view`) tell what is on; `autoxide ai check` tries the settings against the
  provider. New settings `AUTOXIDE_AI_*` and `AUTOXIDE_EMBEDDING_*`.

- AI agents may be limited to their own work: permissions `content.write_own` and
  `content.publish_own` (only entries created by the same user with the same API key), review
  requests (`POST/DELETE /api/v1/content/entries/{id}/request-review`, events
  `content.review_requested` / `content.review_withdrawn`, MCP tool `content.request_review`),
  `ai_generated` on entries and `ai_reviewed` on the published version (only a signed-in person
  can confirm a review), `created_via` in the admin API. MCP tools may accept an alternative
  permission. Requests made with an API key are rate limited per key
  (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`, `…_BURST`).

- Content (`/api/v1/content`, guide in `docs/CONTENT.md`): posts and pages with a draft kept
  apart from the published version, a JSON body (ProseMirror / Tiptap) checked against an
  allowlist, slugs made from titles (`Đường đến Đà Lạt` → `duong-den-da-lat`), revisions with
  restore, scheduled publish and take-down, nested pages with addresses computed by the
  server and automatic `301` redirects when they change (`410` once removed), manual
  redirects, translations (one entry per language, no fallback), categories, tags, author
  profiles, menus, SEO fields. Public endpoints for the website (`/public/resolve`, posts,
  menus, terms, authors, sitemap, feed) never return drafts and answer `304` to a matching
  `ETag`; signed draft preview links. `If-Match` is required on every change. Events carry
  the addresses to purge from a website cache. Permissions `content.read`, `content.write`,
  `content.publish`, `content.manage`; nine MCP tools (`content.list` … `content.resolve`)
  that take the entry `version`. New settings `AUTOXIDE_CONTENT_*`.
- Text search ignores accents as well as case: `da lat` finds `Đà Lạt` (users, media items,
  media tags), the same on SQLite and PostgreSQL.
- Media library (`/api/v1/media`, guide in `docs/MEDIA.md`): images, video, audio and
  documents in nested folders. Uploads are checked against their real type (first bytes),
  raster images get their width and height, duplicates are reported. Items can be public
  (no sign-in needed) or private; content is served with byte ranges, `HEAD`, `304`, safe
  headers (`inline` only for raster images, `nosniff`, a sandboxing CSP) and long caching
  for versioned URLs. Edits need `If-Match`; content can be replaced keeping the id. A
  trash with restore and purge, and daily cleanup jobs. Permissions `media.read`,
  `media.write`, `media.purge`; MCP tools `media.list`, `media.get`, `media.update`,
  `media.trash`. New settings `AUTOXIDE_MEDIA_MAX_BYTES`, `AUTOXIDE_MEDIA_ALLOWED_TYPES`,
  `AUTOXIDE_MEDIA_TRASH_DAYS`.
- Media: smaller copies of JPEG, PNG, GIF and WebP images made in the background
  (`variants`, `?w=` on the content URL; widths set by `AUTOXIDE_MEDIA_IMAGE_WIDTHS`), tags
  next to folders (`tags` on upload and edit, `?tag=` filter, `/api/v1/media/tags`), and the
  MCP tools `media.upload` (base64, up to 1 MiB) and `media.list_tags`.
- Every `GET` that returns an `ETag` answers `304 Not Modified` to a matching
  `If-None-Match`.
- For module authors: lists sorted by another column (`pagination::fetch_page_by`), shared
  text search (`search`), public endpoints (`Option<Principal>`), mandatory `If-Match`
  (`IfMatch::required`, `428`), and byte-range reads and prefix listing in `Storage`.

- Session management: `GET /api/v1/auth/me/sessions` lists your signed-in devices,
  `DELETE /api/v1/auth/me/sessions/{id}` signs one out, `DELETE /api/v1/auth/me/sessions`
  signs out all (or all but `keep`); administrators use
  `DELETE /api/v1/auth/users/{id}/sessions`. Token responses carry `session_id`.
- Account deletion (`DELETE /api/v1/auth/me`, `DELETE /api/v1/auth/users/{id}`) and personal
  data export (`GET /api/v1/auth/me/export`) across every module. Modules join through
  `autoxide_kernel::privacy` (`PersonalData`, the `auth.user_deleted` event).
- User search: `GET /api/v1/auth/users?search=…` (and `search` in the `list_users` MCP
  tool) finds users by email or name, in any case.
- Protection against overwriting someone else's change: `GET` of a user, role, note or
  webhook endpoint returns an `ETag`; send it in `If-Match` with `PATCH`/`PUT` to get `412`
  (code `precondition_failed`) instead of overwriting a newer version.
- MCP tools for roles, API keys, the audit log and files (content as base64, up to 1 MiB).
- Webhooks (`/api/v1/webhooks`, permission `webhooks.manage`): events are sent to your
  `https://` endpoints, signed following Standard Webhooks, retried for about two hours,
  with a delivery log. New settings `AUTOXIDE_WEBHOOKS_ALLOW_HTTP` and
  `AUTOXIDE_WEBHOOKS_TIMEOUT_SECONDS`.

### Changed

- Deleting a post or page needs the new permission `content.delete`; `content.publish` no
  longer includes it. The `admin` role has every permission, as before; give
  `content.delete` to other roles that should delete.
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
- Webhooks are sent only to public addresses: private, local and link-local addresses
  (including names that resolve to them, checked when connecting) are refused unless
  `AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS=true`; the delivery log keeps only a short reason.
- Responses holding a one-time secret (new API keys, tokens, recovery codes, the TOTP secret)
  carry `Cache-Control: no-store` and are never stored for `Idempotency-Key`; a repeat gets
  `409 idempotency_result_not_kept`. The MCP tool `auth.create_api_key` is marked the same way.
- Uploads larger than `AUTOXIDE_HTTP_BODY_LIMIT_BYTES` sent with `Idempotency-Key` are no longer
  held in memory before the handler checks permissions.
- Deleting an account also clears the email and name from the audit entries about it, and the
  erasure event is retried until every module is done instead of giving up after ten tries.
- Two last administrators removing each other at the same time can no longer leave none
  (Postgres).
- A tampered list cursor of the wrong type answers `invalid_cursor` on both databases instead
  of a server error on Postgres.
- Media: an SVG with anything that can run (`<script>`, `on…=` attributes, `javascript:` links,
  embedded HTML) anywhere in the file is refused (`media.svg_active_content`); alt text
  suggestions always send a re-encoded copy without metadata; images whose file is far larger
  than their pixels get no smaller copies; a suffix `Range` on an empty file answers `416`.
- Content:
  - unpublishing or archiving cancels pending scheduled publishes, which could publish the
    entry again;
  - callers with only `content.publish_own` cannot republish an archived entry
    (`content.archived`), set `published_at`, or publish over an editor's redirect
    (`content.path_has_redirect`);
  - callers with only `content.write_own` always get `ai_generated: true`, may attach only
    their own author profiles and may place pages only under their own pages;
  - redirects and menu links refuse `//host`;
  - public search `q` is capped at 200 characters;
  - the public preview no longer shows internal user ids;
  - setting terms or authors is safe under concurrent requests on Postgres;
  - deleting an account also clears who asked for review and who reviewed AI content, while
    `ai_reviewed` stays shown.

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
