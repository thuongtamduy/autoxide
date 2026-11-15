# Changelog

Notable changes to Autoxide. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html): while the version is `0.x`, a
minor release (`0.2.0`) may contain breaking changes, which are always listed under
**Changed** or **Removed**. The HTTP API itself never breaks within `/api/v1`: replaced
endpoints are marked deprecated with a sunset date first.

## [Unreleased]

### Added

- Foundations for serving many customers (tenants) from one installation: the control-plane tables
  `kernel_tenants`, `kernel_placements` and `kernel_tenant_domains` (the existing data belongs to the default tenant),
  and the settings `AUTOXIDE_TENANCY_*`. The default mode `single` changes nothing; `multi` is refused until every
  module keeps its data per tenant.

- `forms`: large CSV exports in the background. `POST /api/v1/forms/{id}/submissions/exports` answers at once;
  `GET /api/v1/forms/exports/{export_id}` gives the state and, once ready, a signed download link that needs no
  sign-in and expires (`AUTOXIDE_FORMS_EXPORT_LINK_MINUTES`). The file has the same content as the existing
  export. It is kept `AUTOXIDE_FORMS_EXPORT_KEEP_HOURS`, and deleted with the submissions of an address or with
  the form. New event `forms.submissions_exported`.
- `auth`: when an account's email changes, the old address gets a mail saying so, with a token (valid 7 days)
  to report a change the owner did not make. `POST /api/v1/auth/email-change/report` signs the account out
  everywhere, removes its password, passkeys, API keys and two-step sign-in, gives the old address back when it is
  free, and sends a password-reset mail there. New setting `AUTOXIDE_AUTH_EMAIL_CHANGE_REPORT_URL`, new event
  `auth.email_change_reported`.
- `auth`: administrators with the new permission `auth.passkeys.manage` see and remove the passkeys of other
  users (`GET`, `DELETE /api/v1/auth/users/{id}/passkeys`, `DELETE …/passkeys/{passkey_id}`); the event
  `auth.passkey_removed` then records `by_administrator`. `autoxide user remove-passkeys --email …` does the same
  from the server for an only administrator who lost their devices.
- `comments`: comments may be written in a small part of Markdown (`format: markdown`: bold, italics, links,
  quotes, code), rendered to safe HTML from a fixed list of tags (raw HTML shows as text, images as their text,
  only `http(s)` links). Plain text stays the default and old comments show as before; Markdown links count
  towards the link limit. New setting `AUTOXIDE_COMMENTS_MARKDOWN` (on), `markdown` in the public settings,
  `format` in every comment.
- For module authors: `autoxide_kernel::tenancy::{TenantScoped, ScopeExt}` — an entity of a tenant-level table
  implements `TenantScoped` (one line naming its `tenant_id` column) and every query of the service adds
  `.scoped(&tenant)` (`Select`, `SelectTwo`, `SelectTwoMany`, `UpdateMany`, `DeleteMany`, and validated
  `update` / `delete` of one row). `Module::control_plane_tables` declares the few tables shared by the whole
  platform. testkit: `schema_tenancy_violations` checks a migrated database (every other table has
  `tenant_id` NOT NULL without a default; unique keys and indexes start with it, except keys made only of UUIDs
  and sweep indexes named `…_sweep_idx`); `cargo xtask check` refuses an entity without `tenant_id`.
- Settings per tenant: `GET /api/v1/kernel/settings`, `PUT` and `DELETE /api/v1/kernel/settings/{name}` (new
  permission `kernel.settings.manage`) and the MCP tool `kernel.list_settings`. Modules declare what they take
  (`Module::tenant_settings`) and read it with `autoxide_kernel::tenancy::TenantSettings`; unset settings fall back
  to the server-wide configuration. Secret values are stored encrypted and never returned, logged or put in events
  (new event `kernel.setting_changed` carries the name only). The kernel declares `kernel.public_url`.
- Secrets the server must read back are stored encrypted (AES-256-GCM) with a key derived from
  `AUTOXIDE_SECRET_KEY`: `autoxide_kernel::secret::Sealer` (`SecretKey::sealer`). Rotating the key: new value in
  `AUTOXIDE_SECRET_KEY`, old one in the new setting `AUTOXIDE_SECRET_KEY_PREVIOUS`, then the new command
  `autoxide secrets reseal`. `autoxide doctor` and the startup log say plainly that a backup holds the key when
  `AUTOXIDE_SECRET_KEY` is unset.
- For module authors: data upgrades (`Module::upgrades`, `autoxide_kernel::migration::DataUpgrade`) — a change
  of stored data that needs the configuration, such as encrypting old secrets. Named `u0001_…`, recorded with the
  migrations of the module, run by `autoxide migrate up` right after the migration they name; `serve` refuses to
  start while one is pending.
- For module authors (testkit): `MemoryStorage`, an in-memory `dyn Storage` with `fail_writes` to test storage
  errors.

### Changed

- For module authors: `Authenticator::authenticate` takes a second argument, the `TenantHint` of the request
  (domain and `Autoxide-Tenant` header). `Principal` carries a tenant (`tenant_id`, `in_tenant`, `operator_from`).
  Handlers may take `tenant: TenantContext` (`autoxide_kernel::tenancy`), and the module start context gives the
  `DatabaseRegistry` (`StartContext::tenancy`).
- For module authors: the kernel's helper tables (`<module>_events`, `_jobs`, `_throttle`, `_tokens`,
  `_antispam_nonces`) carry `tenant_id`; a module adds one migration,
  `tenancy::migration::adopt("…", vec![event::tenant_table("<module>"), job::tenant_table("<module>"), …])`, that
  moves the existing rows to the default tenant. Every call that writes or reads them names the tenant:
  `Outbox::record(conn, &tenant, …)`, `JobQueue::enqueue(conn, &tenant, …)` (and `enqueue_database` for a job of
  the whole database), `OneTimeTokens::{issue, consume, recent, revoke, forget}(conn, &tenant, …)`,
  `Throttle::new(module, limit, window)` without a database and `Throttle::{check, hit, record, refund,
  clear}(&tenant, key)`, `Throttle::purge(&database, before)`, `SpamGuard::check(&tenant, …)` and
  `Checked::claim(conn, &tenant, …)`. `Delivered` and `JobContext` carry the tenant (`Delivered::new(tenant, …)`,
  `JobContext::for_tenant` / `for_database`; `JobContext::tenant` is `None` for a job of the whole database).
  `Claim::claim` takes a `ClaimRequest`. `Modules::worker(concurrency)` and `Modules::dispatcher()` no longer take
  a database: they run on every database of the registry, and the worker shares the free slots fairly between
  the tenants that have jobs due. For tests: `TestDatabase::extra_database`.
- For module authors: every kernel trait that reaches the data of a customer names the tenant:
  `PersonalData::export(&tenant, user_id)`, `ResourceResolver::{resolve, resolve_many, children}(&tenant, …)`,
  `BadgeResolver::badges(&tenant, …)`, `Mailer::send(&tenant, mail)`, `TextModel::generate(&tenant, prompt)`,
  `Embedder::embed(&tenant, …)`, `IdempotencyStore::{begin, complete, abandon}(&tenant, …)`,
  `PostingVerifier::is_verified(&tenant, user_id)`, and the matching `ResourceDirectory`,
  `PersonalDataExport` and `PostingVerification` methods. An MCP tool's handler receives the tenant of the
  request first: `Tool::new(name, description, |tenant: TenantContext, principal: Principal, input| …)`. The
  `api` traits of modules take it too: `autoxide_auth::api::{Users, Accounts, AccountRecovery}`,
  `autoxide_media::api::Media`, `autoxide_notes::api::Notes`, `autoxide_content::api::LegacyMenus`.
- For module authors: the trait `autoxide_auth::api::AccountRecovery` has a new method `remove_passkeys`; only
  `auth` implements it.
- Interface flags (`/api/v1/me/flags`) are kept per tenant: the same account in two tenants has two sets.
- Internal, no change in behaviour: the code of `newsletter`, `comments`, `media` and `forms` is grouped by
  feature like `content` and `auth`, and the kernel's files sit in private groups (`contracts`, `data`,
  `security`, `ops`, `values`). Every public path (`autoxide_kernel::ai::TextModel`, …) is unchanged, and a
  test keeps it so.

### Fixed

- `./gate.sh` ran `cargo xtask api-diff` and `cargo xtask sdk --check` before it stopped on errors, so a
  breaking API change or an outdated SDK did not turn the gate red. They now run as steps like the others.

## [0.14.0] - 2026-11-15

### Added

- `cargo xtask api-diff`: the gate refuses breaking changes to `/api/v1` against the last release tag
  (removed operations, new required parameters or fields, removed response fields, narrowed types, removed
  enum values…), each with a stable rule code and the way to do it right. Deprecated operations now carry
  `x-sunset` in the OpenAPI document.
- MCP tools have a contract like endpoints: `autoxide mcp-manifest` prints `docs/mcp-tools.json`, and
  `cargo xtask api-diff` refuses breaking changes to tools (removed tool, new required parameter, a
  read-only tool that now writes…). Modules deprecate a tool with `Deprecation::tool(…)` in
  `Module::deprecations`; `tools/list` says so and every result carries `_meta.autoxide/deprecation` and a
  warning. The module catalog lists a tool's `deprecation`.
- `cargo xtask semver`: lists breaking changes to the Rust contract (`autoxide-kernel`, `autoxide-testkit`)
  against the last release, with `cargo-semver-checks`; run when releasing. It only reports while the
  version is `0.x`, and refuses breaking changes from `1.0` or with `--deny`.
- A typed TypeScript SDK in `sdk/typescript/` (no dependencies; not on npm): `createClient`, typed paths,
  parameters, bodies and results generated from `docs/openapi.json` by `cargo xtask sdk`, `AutoxideError`,
  `paginate()`. The gate checks it matches the contract; each release attaches `autoxide-sdk-X.Y.Z.tgz`.
- Task-by-task guides in `docs/guides/` (English and Vietnamese): a website (public endpoints, `srcset`
  and WebP, ETag / 304), signing in and refreshing, API keys, safe retries, pagination, errors, and
  connecting an AI agent over MCP with every tool and its permission. Their `curl` examples run as tests
  on every build; their TypeScript examples are type-checked against the SDK in CI.
- SDK: `AutoxideError.requestId` (the `X-Request-Id` of the answer).
- For module authors (testkit): `schema_snapshot` (tables, columns, indexes of a SQLite or Postgres
  database, to compare schemas) and `TestAppBuilder::boxed_module`. Every module's real migrations are
  tested up → down → up, on an empty database and with data, on both databases.
- Property tests (`proptest`, tests only) for the input checks: email, pagination cursors, post bodies,
  slugs, CSV export, comment text and links (no HTML but safe links ever comes out).
- A machine-readable access contract: every operation in the OpenAPI document carries `x-permissions`
  (one of them is enough; `[]` for any signed-in caller) or `x-public: true`, and the module catalog gives
  each permission a `risk` (`read`, `write`, `destructive`, `sensitive_data`, `money`). The gate checks
  every endpoint answers 401 without a token and 403 `permission_denied` without the permission.
- For module authors: `Permission::with_risk`, `Risk`, `Tool::for_any_signed_in`; the OpenAPI lint asks
  every handler for `x-permissions` or `x-public`.
- An upgrade test from the last release: `cargo xtask upgrade-test` runs the previous release's Docker
  image, writes sample data through its API (users, tokens, API keys, images, posts, forms, newsletter
  lists, webhooks, menus), then runs this version on the same database and data directory and checks every
  record, the old tokens and a few writes — on SQLite and PostgreSQL. It runs in CI and before every
  release (Linux, needs Docker).

### Fixed

- The OpenAPI document declared `application/json` for error responses; it now declares
  `application/problem+json`, the media type errors are sent with.
- `GET` / `DELETE /api/v1/{comments,forms,newsletter}/data` answered a caller without the permission with
  `422` when `email` was missing; the permission is checked first now (`403`).
- MCP tools of `files` now require `files.upload` or `files.manage`, like their endpoints (a caller without
  them no longer sees them in `tools/list`).
- The OpenAPI document declared the `value` of a form answer as an object; it is any JSON value (a string,
  a number, an array or `true`), as it always was.

### Changed

- For module authors: `TestRequest::send` (testkit) checks every response against the OpenAPI document of
  the app — status, media type and body — and fails the test with the place that differs.
  `TestRequest::unchecked()` skips the check for tests that send out-of-contract answers on purpose.

## [0.13.0] - 2026-11-14

### Added

- Comments as product questions and answers: authors get badges such as `seller` or
  `verified_buyer` (codes in `badges`, computed by the module owning the target); `q` on
  `GET /api/v1/comments/public/threads` searches a target's top-level comments ignoring accents,
  exact-accent matches first; counts carry `thread_count`; `unanswered=true` on the moderation list
  and the MCP tools `comments.unanswered` and `comments.answer`; `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES`
  takes comments on some kinds only from verified accounts (`comments.verification_required`).
- Media: alt text per language — `PUT` / `DELETE /api/v1/media/{id}/alts/{locale}`, `alts` on every
  item, the MCP tool `media.set_alt`, languages from `AUTOXIDE_MEDIA_ALT_LOCALES` (default `en,vi`).
  The content module's public images use the alt text in the post's language when there is one.
- Interface flags per account: `GET` / `PUT` / `DELETE /api/v1/me/flags` remember dismissed hints or the
  last "what's new" seen, for the signed-in user only (at most `AUTOXIDE_USER_FLAGS_MAX`, default 500).
  Flags are part of the personal data export and are deleted with the account.
- For module authors: `Module::resource_events`, `StartContext::badge_source`,
  `StartContext::posting_verifier` and `pagination::fetch_page_tiers` (see `docs/MODULE_GUIDE.md`).
- Website menus (`menus` module, `docs/MENUS.md`): `GET`/`POST /api/v1/menus`, `GET`/`PUT`/`DELETE
  /api/v1/menus/{id}` (`If-Match` required to replace items), and the public
  `GET /api/v1/menus/public/{key}` with `ETag` and caching (`AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS`). Items
  link to addresses or to objects of any module (`ref`), take sub-items by themselves (`auto_children`),
  and carry icons, highlights, dated badges and a featured object. MCP tools `menus.*`; permissions
  `menus.read`, `menus.manage`; `autoxide task menus.import_content_menus` copies the content module's
  menus.
- The resource directory looks up many objects at once in a language (`ResourceResolver::resolve_many`),
  gives pictures (`Resource::image`) and sub-items (`has_children`, `children`). The content module
  registers categories and tags as `content.term` (public path `/<post prefix>/category/<slug>` or
  `/…/tag/<slug>`).

### Deprecated

- The content module's menu endpoints (`GET /api/v1/content/menus`, `GET`/`PUT
  /api/v1/content/menus/{key}`, `GET /api/v1/content/public/menus/{key}`), sunset 2027-05-14: use the
  `menus` module.

## [0.12.0] - 2026-11-13

### Added

- `AUTOXIDE_HTTP_DRAIN_SECONDS` (default `0`): on shutdown the server keeps handling requests
  this long while `/health/ready` answers 503 with code `shutting_down`, so a load balancer
  stops sending it requests before it stops accepting connections. `docs/DEPLOY.md` explains
  rolling updates.
- `SECURITY.md` (and `SECURITY.vi.md`): which versions get security fixes, how to report a
  vulnerability privately, response times and what is out of scope.
- Media: every smaller copy of an image also gets a WebP version, usually 55–60% lighter than
  the JPEG for photos and over 90% lighter than the PNG for images with transparency. Each
  entry of `variants` has `webp_url` (the copy's `url` plus `&format=webp`) and `webp_size`;
  the content module's public `featured_media` and `body_media` carry `webp_url` too. Run
  `autoxide task media.refresh_variants` once to add WebP copies to images uploaded before.
- The `Release` workflow is ready to publish Docker images for `linux/amd64` and `linux/arm64`
  as `ghcr.io/<owner>/autoxide:X.Y.Z` and `:X.Y`, with a signed build provenance and a signed
  SBOM; `docs/DEPLOY.md` shows how to verify them with `gh attestation verify`. It runs once
  the repository is public: this release has no published image yet — build it with
  `docker compose up -d --build`.
- `CODE_OF_CONDUCT.md`: the Contributor Covenant 3.0, with a Vietnamese translation.
- `CONTRIBUTING.md` (and `CONTRIBUTING.vi.md`): setting up a machine, the two gates, the
  rules that matter most, commits and pull requests, and the license of contributions.

### Changed

- The GitHub workflows run again on their own (CI on pushes to `main` and pull requests, a
  daily dependency audit, releases on `vX.Y.Z` tags), but only while the repository is public.
- `autoxide_media::api::MediaVariant` has a new field, `webp_url`.
- Building Autoxide compiles `libwebp` (C), like SQLite: a C compiler was already required.
- `autoxide_kernel::http::health_routes` takes a second argument, the `http::Draining` flag
  that makes `/health/ready` answer 503 while shutting down; pass `Draining::default()` where
  the server is not stopped by a signal (tests).
- `compose.yml` gives the server 40 seconds to stop (`stop_grace_period`) instead of Docker's
  10, so requests in progress get the full `AUTOXIDE_HTTP_SHUTDOWN_SECONDS`.

### Fixed

- The installation guide (`docs/DEPLOY.md`) now says a C compiler is needed to build, and
  generates `AUTOXIDE_SECRET_KEY` with the other secrets, so a server installed by following
  it no longer warns about a missing key; `compose.yml` passes `AUTOXIDE_SECRET_KEY` through.

## [0.11.0] - 2026-11-13

### Added

- `autoxide_kernel::money`: a `Money` type for modules that handle money — an `i64` count of
  the currency's minor unit plus an ISO 4217 currency. Arithmetic returns an error instead of
  overflowing, mixing currencies is refused (`money.currency_mismatch`), and `allocate` /
  `split` divide an amount without losing a unit (100 split 3 ways is 34, 33, 33). In JSON
  the amount is a string: `{"amount": "1999", "currency": "USD"}`.
- `autoxide_kernel::rows`: `lock_one` / `lock` lock rows until the end of a write transaction
  on SQLite and PostgreSQL, and `adjust` changes a balance in one conditional update that
  never goes below a floor or overflows.
- `Database::write_with_retry` runs a write again when PostgreSQL reports a deadlock or a
  serialization failure, or SQLite stays busy.
- `autoxide_kernel::reference`: a client-chosen `reference` that is kept with a record makes
  a command run only once, however late it is sent again; different content under the same
  reference fails with `reference_reused` (422).
- The audit log is a hash chain: `autoxide doctor` reports the first entry that was changed,
  removed or inserted afterwards (`audit.chain`). Existing entries are chained when the
  database is migrated; deleting an account still removes its personal data.
- The module guide has a section on money and balances.

### Changed

- Release builds check integer overflow (`overflow-checks = true`): an overflow stops the
  request with an error instead of silently wrapping around.

## [0.10.0] - 2026-11-13

### Changed

- `forms`, `comments` and `newsletter` check email addresses the same way: at most 254 bytes in
  all, 64 bytes before the `@`, a domain with at least one dot. Before, `newsletter` accepted
  addresses up to 320 bytes (which mail servers refuse) and `comments` counted characters
  instead of bytes.

## [0.9.0] - 2026-11-13

### Added

- `autoxide task`: lists and runs maintenance tasks that modules declare (`Module::tasks`).
  The first one, `media.refresh_variants [dry_run=true]`, queues new smaller copies for images
  uploaded before `AUTOXIDE_MEDIA_IMAGE_WIDTHS` changed.

### Changed

- `autoxide-testkit`: every module's HTTP tests use `TestApp`.
- Module contract: the types modules used to build with struct literals now have constructors
  and builders (`Mail::new(..)`, `Prompt::new(..)`, `EnqueueOptions::default().unique_key(..)`,
  `Submission::new(..)`, `Delivered::new(..)` and others) and are `#[non_exhaustive]`: build
  them with those instead of struct literals.

## [0.8.0] - 2026-11-13

### Added

- `autoxide doctor [--json]`: checks the configuration, the database, the migrations, the
  signing key, file storage (writes, reads back and deletes a test file), the SMTP server and
  whether an administrator exists, and says how to fix each problem.
- Module contract: `Module::checks` declares checks for `autoxide doctor` (default: none).
- `cargo xtask new-module <module> <entity>` creates a new module from the `notes` sample.
- `autoxide-testkit`: `TestApp` builds modules the way the binary does on a test database and
  calls them through REST and a real MCP client.
- `content`: a public entry carries `body_media`, the images used in its body (in order of
  appearance, without repeats) with their size and smaller copies, so a website can build
  `srcset` and reserve room for them. An image that is no longer public is left out.

### Changed

- Module contract: public kernel types that will grow (configuration, `Problem`, `Created`,
  `BodyLimit`, `Deprecation`, `Schedule`, `ErrorKind`, `Language`, `SortValue` and others) are
  `#[non_exhaustive]`: build them with their constructors and match their enums with a `_` arm.
  `docs/MODULE_GUIDE.md` now lists the libraries that are part of the contract and the rules
  for changing it.

### Fixed

- Files: a failed upload of a small file (unwritable directory, storage refusing it) no longer
  crashes the request with a panic; it returns an error.

## [0.7.0] - 2026-11-12

### Added

- `autoxide user reset-mfa --email …`: the server operator turns off two-step sign-in of an
  account, for an only administrator who lost their device.
- Files: a daily job removes stored content that no file record points to any more (older than
  a day), such as an upload cut off between storing the content and saving its record (new
  migration).

### Fixed

- Webhooks: `autoxide jobs retry` on a failed `webhooks.send` job sends the delivery again
  instead of doing nothing.

### Security

- `POST /api/v1/auth/password-reset` answers in the same time whether or not the email has an
  account: a background job looks the account up and sends the mail a moment later.

## [0.6.0] - 2026-11-12

### Changed

- **Breaking:** `POST /api/v1/auth/me/mfa/totp` (start two-step sign-in) needs
  `{"current_password": "…"}`, like adding a passkey or an API key: a stolen access token alone
  can no longer tie someone else's authenticator app to the account.

### Fixed

- Jobs: the worker no longer waits for a whole batch before taking new jobs: each module runs up
  to `AUTOXIDE_JOBS_CONCURRENCY` jobs at once and a slow job (AI call, slow webhook) only holds
  its own slot. A job whose handler panics counts as a failed attempt; a job whose process died
  on its last attempt goes to the dead-letter queue instead of being retried forever.
- Jobs queued with a `unique_key` that already exists inside a transaction no longer break that
  transaction on Postgres.
- Events: a subscriber that panics or takes longer than 50 seconds counts as a failed delivery
  (retried later) instead of stopping event delivery; an unreadable event row is marked failed
  instead of blocking every later event; a claim is taken with the current time, so a slow
  batch can no longer let a second server deliver the same event.
- Files: an upload cut off midway (timeout, client gone) aborts the partial S3 multipart upload;
  a download the client already has (`If-None-Match`) answers 304 without reading the stored
  content.
- Content: several entries created at the same moment with the same title (automatic slug) no
  longer fail with `content.slug_taken` on Postgres.
- Faster cleanup and filtering: the hourly sign-in cleanup uses indexes and deletes in batches of
  1,000 rows; the audit log filters by `module` through an index (new migrations in `auth` and
  `audit`, applied by `autoxide migrate`).

### Security

- Resetting or changing a password also ends, in the same transaction, everything opened with
  the old password: a pending email change (its confirmation link stops working), a passkey
  registration not finished yet, and a sign-in waiting for its two-step code. A reset also removes
  passkeys and API keys in that same transaction.
- A session or API key is granted only if the password just checked is still the current one
  and the account is still active: a password reset that lands between the check and the grant
  wins.
- Confirming a new email address voids password-reset mails sent to the old one.
- Disabling or deleting a user while someone changes their roles no longer risks a deadlock on
  Postgres.
- Requests whose client address is an IPv4 address written as IPv6 (`::ffff:a.b.c.d`, when the
  server listens on `[::]`) are rate-limited per address again, instead of all sharing one
  limit.
- `Idempotency-Key`: a temporary refusal (429, 408) is no longer stored and replayed; a request
  that timed out or crashed after it may have taken effect answers a retry with
  `409 idempotency_result_unknown` instead of running again; responses over 256 KiB are not
  stored (`409 idempotency_result_too_large` on a retry); `If-Match` is part of the request's
  fingerprint and `ETag` is replayed.
- The NUL character (`\u0000` in JSON, `%00` in a URL) is refused everywhere with
  `400 invalid_character`; before, Postgres answered 500 while SQLite stored it.
- `/health/ready` asks the database at most once a second, however often it is called.
- The `method` label of HTTP metrics only takes standard methods (others count as `other`).
- Webhooks: creating an endpoint and rotating its secret answer with `Cache-Control: no-store`;
  events about endpoints carry only the origin of the URL (paths and queries often hold
  tokens); more IPv6 forms that embed private IPv4 addresses (6to4, IPv4-compatible) and the
  remaining special ranges are refused as targets.
- Files: names with invisible formatting characters (such as right-to-left override) are
  refused.
- Mail: only a permanent SMTP refusal that blames the recipient address (enhanced status
  `5.1.x` / `5.2.x`) emits `mail.delivery_failed`; other permanent errors (wrong SMTP password,
  sender not allowed, sending quota) are retried and then land in the dead-letter queue, so a
  misconfigured SMTP server no longer gets every newsletter subscriber blocked.
- Backups and the scratch copy made during a restore are created readable by their owner only
  (`0700` directories, on Unix).

## [0.5.0] - 2026-11-12

### Added

- Installed-module catalog for admin screens and AI agents: `GET /api/v1/kernel/modules` and
  `GET /api/v1/kernel/modules/{name}` (with migration status), MCP tools `kernel.list_modules` and
  `kernel.get_module`, permission `kernel.modules.read`.

## [0.4.0] - 2026-11-12

### Added

- `AUTOXIDE_HTTP_TRUSTED_PROXIES`: list the addresses and ranges of your proxies instead of
  counting them. The client-address header is then read only from a trusted proxy, and the
  client is the last address in it that is not a trusted proxy — safe even when visitors can
  reach one of your proxies directly.
- The server warns at startup when `AUTOXIDE_SECRET_KEY` is not set.

### Changed

- **Breaking:** hashed IP addresses differ per module (forms, newsletter, comments): one
  visitor's records can no longer be linked across modules. Hashes stored before this version
  stop matching new ones, so comment bans by network fingerprint made before it no longer
  apply — ban again from a newer comment.

### Security

- Newsletter: a public sign-up only checks the data and the anti-spam token, then queues a job
  that stores it and sends the confirmation mail. The answer now takes the same time whether
  the address is new, already subscribed or blocked. The confirmation mail leaves a moment
  later, once the job worker picks the job up.
- Comments: a display name written entirely in Cyrillic or Greek letters that look like Latin
  ones (`аdmіn`) counts as the Latin name — it can no longer pass the reserved-name list or take
  another person's name. Existing profiles are re-keyed by a migration.

## [0.3.0] - 2026-11-12

### Changed

- **Breaking:** a signed-in caller skips the anti-spam checks for anonymous visitors only with
  `forms.submit` (forms) or `newsletter.subscribe` (newsletter sign-ups) — give these to your
  website server's API key. Other accounts must send a form token like anyone else.
- Changing a form's `notify_emails` also needs `forms.read`; turning on `new_posts: send` for a
  newsletter list needs `newsletter.send`; editing a scheduled campaign puts it back to draft.
- `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` counts sign-up requests that may send a mail, not
  mails sent.
- Comments: `PATCH /api/v1/comments/public/comments/{id}` also returns `status` (an edit of a
  shown comment may wait for approval again); deleting an unknown id answers like a real delete.

### Security

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
- Comments: a few anonymous reports can no longer get an old comment deleted by the daily
  clean-up (only comments that were never shown expire); editing an approved comment goes
  through moderation again; trust from earlier approval counts only comments still shown; names
  refuse invisible and mixed-script characters; races around replies and hard deletes fixed.
- A form token sent again no longer counts towards the per-address and per-email limits, so
  nobody can block someone else's email that way.
- Forms: email addresses are checked in bytes like the mail library, so a long Unicode address
  no longer breaks admin notifications (or gives a server error on PostgreSQL); erasing by email
  finds every answer with that address; submissions to a form deleted meanwhile are refused.
- Newsletter: someone who unsubscribed (or never confirmed) must confirm by mail even on
  single opt-in lists; the answer to a sign-up no longer depends on whether a mail was queued.
- Mail templates are filled in one pass, so `{…}` in submitted text no longer breaks them.
- Shared resource lookups no longer expose titles of unpublished content; the audit log keeps
  only a hash of the address of a mail refused for good.

## [0.2.0] - 2026-11-12

### Added

- Shared building blocks for anything the public sends (used by forms, newsletters and
  comments): a signing key for the whole server (`AUTOXIDE_SECRET_KEY`, optional — generated
  once and kept in the database when unset); form tokens from
  `GET /api/v1/antispam/token?scope=…` with a trap field, a minimum fill time, rate limits by
  hashed IP and email, and an optional ALTCHA proof-of-work challenge
  (`AUTOXIDE_ANTISPAM_*`, off by default); `AUTOXIDE_HTTP_PUBLIC_URL` for absolute links in
  mail; `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` for a CDN in front of a website server in front
  of the API. Raw IP addresses are never stored, only keyed hashes.
- Mail: `Reply-To`, `List-Unsubscribe` and `List-Unsubscribe-Post` headers, a key that stops
  the same mail from being queued twice, and an event `mail.delivery_failed` when the
  receiving server refuses a mail for good.
- The audit log (`actor_kind`) and webhooks (`data.actor.kind`) tell apart changes made by
  the system, by a user, and by an anonymous visitor (`public`).
- Comments (`/api/v1/comments`, guide in `docs/COMMENTS.md`): two-level comments on any
  allowed target type (checked through the resource directory, e.g. `content.entry`), from
  signed-in users with a display name of their own or from guests (optional) with an edit
  token. A fixed moderation pipeline (closed target, length, duplicates, bans, banned and
  suspect words, too many links, anti-spam) with `pre`, `first_time` or `post` moderation per
  target type, for guests and users separately; approval history counts by account only.
  Plain-text bodies returned as escaped HTML with `http(s)` links marked
  `nofollow ugc noopener`. Author edits within a time window, soft delete that keeps the
  thread, reports that hold a comment at a threshold, bans by user, email or hashed IP with
  an optional end, open / closed targets, counts for up to 100 targets at once, opt-in reply
  mails to verified addresses, export and erasure by email, automatic deletion of old pending
  and spam comments. Permissions `comments.post`, `comments.read`, `comments.moderate`,
  `comments.manage`, `comments.export`; four MCP tools. New settings `AUTOXIDE_COMMENTS_*`.
- Newsletters (`/api/v1/newsletter`, guide in `docs/NEWSLETTER.md`): lists with titles in
  English and Vietnamese, public sign-up through the same anti-spam checks as forms (the
  answer never tells whether an address was already known), double opt-in confirmed by a
  `POST` from the website, one-click unsubscribe (RFC 8058 `List-Unsubscribe` headers on every
  campaign mail) and a preferences page by token. Campaigns: drafts, scheduling, sending in
  batches at a configured rate, pause, resume and cancel, a test mail; re-runs never send
  twice. A suppression list fed by permanent delivery failures, `POST /bounces` (hard, soft
  with a threshold, complaint) and manual entries. Optional campaigns for new posts of the
  content module. Advertising mails get a configurable subject label and a daily limit per
  address. Export and erasure by email, import with a consent source. Permissions
  `newsletter.read`, `newsletter.manage`, `newsletter.send`, `newsletter.export`; six MCP
  tools. New settings `AUTOXIDE_NEWSLETTER_*`.
- `autoxide openapi` (and the test that keeps `docs/openapi.json` in sync) now fails when two
  modules declare a schema with the same name but different content, instead of silently
  keeping one.
- Forms (`/api/v1/forms`, guide in `docs/FORMS.md`): forms defined through the API with a
  closed list of field types (text, email, phone, textarea, number, select, radio, checkbox,
  date, hidden, consent), labels in English and Vietnamese, and versions so old submissions
  keep their labels. Public endpoints show a form and take submissions: every answer is
  checked on the server and every problem reported at once; a trap field, a signed form
  token, an optional ALTCHA challenge and rate limits by hashed IP and email hold spam back,
  and the answer is the same whether a submission is accepted, held or dropped. Sending the
  same form token again returns the first answer. An inbox with statuses, accent-insensitive
  search and CSV export (guarded against formula injection), notification mail to
  administrators with `Reply-To` set to the sender, export and erasure by email, automatic
  deletion after a retention period, consent proof. Permissions `forms.read`,
  `forms.manage`, `forms.review`, `forms.export`; seven MCP tools, including creating and
  changing forms. New settings `AUTOXIDE_FORMS_*`.
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

- Two built-in modules, `kernel` and `antispam`, are always registered: `autoxide migrate`
  lists them and creates their tables. The login throttle and the password reset and email
  verification tokens of `auth` move to shared tables during `autoxide migrate`; links in
  mails already sent keep working.
- With `AUTOXIDE_HTTP_CLIENT_IP_HEADER`, the client address is read
  `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` entries from the end of the header, across all its
  lines (the default `1` keeps the previous behaviour).
- A mail refused for good by the receiving server (SMTP `5xx`) is no longer retried.
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

[Unreleased]: https://github.com/thuongtamduy/autoxide/compare/v0.14.0...HEAD
[0.14.0]: https://github.com/thuongtamduy/autoxide/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/thuongtamduy/autoxide/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/thuongtamduy/autoxide/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/thuongtamduy/autoxide/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/thuongtamduy/autoxide/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/thuongtamduy/autoxide/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/thuongtamduy/autoxide/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/thuongtamduy/autoxide/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/thuongtamduy/autoxide/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/thuongtamduy/autoxide/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/thuongtamduy/autoxide/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/thuongtamduy/autoxide/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/thuongtamduy/autoxide/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/thuongtamduy/autoxide/releases/tag/v0.1.0
