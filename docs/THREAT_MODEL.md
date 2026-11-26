# Threat model of the core

*Tiếng Việt: [THREAT_MODEL.vi.md](THREAT_MODEL.vi.md)*

> **What this is:** the maintainers' model of what an attacker could aim at in an Autoxide server, which controls
> exist today and what is still open. It was written with AI assistance from the code and the decision log, as input
> for a round of penetration testing by people outside the project. It is **not** an audit and does not claim that
> Autoxide is free of vulnerabilities. To report a problem, follow [`SECURITY.md`](../SECURITY.md).

- **Date:** 2026-11-26, `main` of the `0.x` series before 1.0
- **Roadmap item:** step 71, G13 ([`docs/ROADMAP.md`](ROADMAP.md))
- **Builds on:** the [November 2026 review](security/review-2026-11.md) (code review and black-box test) and the
  [November 2026 security and performance test](security/pentest-2026-11/README.md) (29,566 cases on both databases)
- **Method:** [STRIDE](https://learn.microsoft.com/en-us/azure/security/develop/threat-modeling-tool-threats) per
  trust boundary — **S**poofing, **T**ampering, **R**epudiation, **I**nformation disclosure, **D**enial of service,
  **E**levation of privilege

## How to read this document

Each of the eight boundaries below has the same parts:

- **Assets** — what an attacker wants at that boundary.
- **Entry points** — where data crosses it.
- **Trust assumptions** — what the design takes for granted. A test that breaks an assumption is a finding.
- **STRIDE table** — threat → existing control → residual risk or open item. A control names where it lives: a code
  path (relative to the repository root), a decision ID (`D-…` in [`docs/DECISIONS.md`](DECISIONS.md) or
  `docs/decisions/D-….md`) or a gate rule of `cargo xtask check`. A control marked **(to verify)** is described by
  the design or the documentation but was not traced end to end in the code for this document; the list is at the
  end, in [Controls to verify](#controls-to-verify).

Findings of the earlier rounds keep their IDs: `H-`, `M-`, `L-`, `I-`, `P-` (review) and `PT-` (test).

## System overview

```text
               browsers, apps, AI agents, partner servers, payment / carrier / bank callbacks
                                         │  HTTPS (TLS ends at a reverse proxy)
  ┌──────────────────────────────────────▼───────────────────────────────────────────────┐
  │ (1) HTTP layer: request id → security headers → log → limits → CORS → tenant → auth  │
  │     REST /api/v1/…  ·  OpenAPI  ·  SSE / WebSocket  ·  public routes  ·  webhooks in │
  │ (2) MCP endpoint /mcp, /mcp/<module>                                                 │
  ├──────────────────────────────────────────────────────────────────────────────────────┤
  │ (3) kernel: Principal and permissions, tenancy, throttle, idempotency, outbox, jobs, │
  │     config and secrets, seal, outbound address filter, storage and AI traits         │
  ├──────────────────────────────────────────────────────────────────────────────────────┤
  │ (4) modules (one crate each, own tables, talk through traits and events)             │
  ├──────────────────────────────────────────────────────────────────────────────────────┤
  │ (5) databases: control database + placements (SQLite or PostgreSQL), file storage    │
  └───────────────┬───────────────────────────────────────────────────┬──────────────────┘
                  │ (6) placement: tenants on shared or own databases │
                  ▼                                                   ▼
  (7) external providers: payments, bank feed, carrier, SMS,   (8) AI providers chosen by the
      e-invoice, mail, outbound webhooks, web push                 installer (TextModel / Embedder)
```

**Modes.** `AUTOXIDE_TENANCY_MODE=single` serves one default tenant and ignores `Host` and the tenant header.
`multi` serves many tenants from one process; this is the mode with the most boundaries and the one to test.

### Actors

| Actor | Credential | What they are trusted with |
| ----- | ---------- | -------------------------- |
| Anonymous visitor | none | public routes of one tenant (storefront, published content, forms, comments, public search) |
| Customer (buyer) | access token of a customer account | their own records; never staff data |
| Tenant staff | access token, optionally TOTP / passkey; permissions from roles, possibly scoped to an organization unit | what their permissions allow, inside their tenant and branch |
| Tenant administrator | as staff, holding admin-equivalent permissions | everything in their tenant; **not** other tenants (see [`SECURITY.md`](../SECURITY.md), "What is not a vulnerability") |
| Machine / AI agent | API key `axk1_…` (scope ∩ owner's current rights, optional IP fence) | the key's scope; never more than the owner |
| SCIM client | SCIM token bound to its creator's rights | provisioning within its creator's reach |
| Platform staff | account of the platform tenant with `kernel.tenants.*` permissions | operating tenants; entering a tenant only by a time-limited support grant |
| Parent tenant | delegation | working in a descendant tenant, at a node, with a role's permissions |
| Provider | per-tenant webhook URL + provider signature | reporting payments, bank credits, shipment status |
| Operator | shell on the server, environment, database | everything (out of scope) |
| External module author | code compiled into the binary | everything the process can do (see boundary 4) |

### Global assumptions

- TLS is terminated by a reverse proxy or load balancer in front of the server; the server itself speaks HTTP
  ([`docs/DEPLOY.md`](DEPLOY.md)). `AUTOXIDE_HTTP_CLIENT_IP_HEADER` / `AUTOXIDE_HTTP_TRUSTED_PROXIES` are set only
  when such a proxy exists (`crates/kernel/src/config/http.rs`).
- `AUTOXIDE_SECRET_KEY` and `AUTOXIDE_AUTH_TOKEN_SECRET` are set to random values in production; the example values
  of `.env.example` are refused at startup (H-3; `crates/kernel/src/security/secret.rs`, `modules/auth/src/config.rs`).
- The database and the file store are reachable only by the server and the operator.
- The code is memory-safe Rust: `unsafe_code = "forbid"` for every workspace crate (`Cargo.toml`); release builds
  keep overflow checks (an overflow is a 500, not a wrap — PT-5).

---

## 1. Client ↔ HTTP layer

**Assets:** availability of the API; the mapping request → tenant; credentials in transit; tenant data in responses;
browser sessions of tenants' websites.

**Entry points:** every REST route under `/api/v1/` (`docs/openapi.json`, about 1,300 operations); public routes
(`security((), ("bearer" = []))`); `/api/v1/openapi.json`, `/api/docs` (opt-in), `/llms.txt`; `/health/*`;
`/metrics` (opt-in); `GET /api/v1/kernel/events/stream` (SSE) and `/events/ws` (WebSocket); inbound provider
webhooks (boundary 7); headers `Host`, `Origin`, `Autoxide-Tenant`, `Authorization`, `Idempotency-Key`,
`If-Match`, the client-address header.

**Trust assumptions:**

- The tenant of an authenticated request comes **from the credential**, never from `Host` or a header alone
  (`crates/kernel/src/data/tenancy/request.rs`).
- A `Host` maps to a tenant only when that tenant has **verified** the domain by DNS TXT
  (`crates/kernel/src/data/tenancy/domains.rs`).
- Public routes only return data the tenant has published; choosing another tenant for a public request (by host,
  header or `?site=`) is allowed and exposes only that tenant's public data.

| STRIDE | Threat | Existing control | Residual / open |
| ------ | ------ | ---------------- | --------------- |
| S | Pretend to be another tenant by sending its `Host` or `Autoxide-Tenant` with one's own credential | Credential's tenant compared with header and verified domain → `tenant_mismatch`; only platform staff with `kernel.tenants.operate`, a delegation or a support grant may enter another tenant (`data/tenancy/request.rs`, `modules/auth/src/delegation.rs`, `modules/auth/src/support.rs`) | Tested on both databases in P14 and the 2026-11 test (IDOR, TOKEN groups) |
| S | Claim a domain one does not own | DNS TXT `_autoxide-challenge.<domain>`; daily recheck, 3 misses → `failed`; one `verified` row per domain (partial unique index, P-4, D-20261125-domain-claims) | Test that deterministically hits the "lost race" branch is open (P16) |
| S | Spoof the client address to dodge per-address limits | Address read from the configured header only, counted from the trusted proxies (`http/client.rs`); IPv4 address or IPv6 /64 as the key (`throttle::address_key`, PT-4, D-20261125-address-keys, gate `address-key`) | Gate only catches one textual pattern (P17) |
| T | Cross-site requests from a browser page | CORS: allow-list `AUTOXIDE_HTTP_CORS_ORIGINS`, or `https://<verified domain>` of the same tenant only (`http/cors.rs`, D-20261125-tenant-cors); credentials are bearer headers, not cookies | `*` is a documented misconfiguration and logs a warning (L-5) |
| T | Replayed or doubled writes (retries, agents) | `Idempotency-Key` stored per caller and request; other body → 422; in flight → 409 (`security/idempotency.rs`, `http/idempotency.rs`, D-058); optimistic concurrency with `If-Match` | — |
| T | Injection through query parameters, cursors, sort keys, bodies | SeaORM with bound values; raw statements only through `database::statement` (D-20261121-raw-statement); cursors carry values, the server chooses the column; `ValidJson<T>` + service validation (D-083); NUL in URL refused (`http/mod.rs`) | 8,061 INPUT cases per database passed (2026-11 test) |
| R | A caller denies having made a request | Request id on every response; per-tenant request log with user, API key, actor kind, route template, status, `Idempotency-Key` (`ops/request_log.rs`, D-202); audit chain of every change (boundary 4) | Public (unauthenticated) requests are not in the request log by design |
| I | Error bodies, headers or logs leak internals or secrets | RFC 9457 `Problem` with stable `code`, no stack or SQL text; logs record the route template, never the raw path, query or body (`http/log.rs`); security headers on every response incl. CSP `default-src 'none'; sandbox` (`http/security_headers.rs`, D-320) | `/metrics` on a public port is a documented misconfiguration (warning at start) |
| I | Learn which records or fields exist without permission | Shared permission gate reads `x-permissions` and answers 403 before body parsing, `If-Match` and lookups (`http/permission_gate.rs`, PT-13/14, D-20261126-permission-gate) | Endpoints with `x-permissions: []` (any signed-in caller) still validate first |
| I | Read drafts or private data through public routes | Public lists, search, feed, sitemap and slug filter on published state; preview tokens are HMAC-signed and compared in constant time (`modules/content/src/publishing/preview.rs`) | PT-11 (taxonomy terms not yet used by a published entry) open (P17) |
| D | Large or deep bodies, compression bombs | Body limit per route (`http/body_limit.rs`, D-066); request bodies with `Content-Encoding` are not inflated (review P14); JSON depth handled by the parser (2026-11 DOS group) | — |
| D | Slow or idle connections | Header read timeout 30 s, idle connections closed, `AUTOXIDE_HTTP_MAX_CONNECTIONS` (M-5, L-10; `http/server.rs`) | — |
| D | Request floods by one client, one key or one tenant | GCRA per client address (`http/rate_limit.rs`), per API key, per tenant from its plan (`http/tenant_rate_limit.rs`, D-20261126-tenant-rate-limit) | Volumetric DoS is out of scope (`SECURITY.md`) |
| D | Expensive public search | Concurrency cap per process, per-address limit and a time limit (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_*`; D-20261126-idle-polling) | Each search still scans with `LIKE`; full-text index behind a trait is open (PT-15, P17) |
| D | Evict the `domain_site` cache by random `Host` values | Cache bounded at 10,000 entries | Eviction policy is open (P16): real domains may hit the database on every request under such a flood |
| E | Call an endpoint without the right permission | `Principal::require` in every handler (rule 7) plus the shared `x-permissions` gate; 2,691 PERM cases per database passed | — |
| E | Use the event stream to see other tenants' or branches' events | `StreamHub` filters by tenant, permission and reach (`event/stream.rs`, D-378); one connection limit per user | WebSocket endpoint does not check `Origin` yet (low: auth is a header a browser cannot set on a WebSocket) (P16) |

## 2. API and MCP endpoint

**Assets:** the same data and actions as REST; the permission model; the parity promise "an MCP tool never does
more than REST with the same permission" (CLAUDE.md rule 8).

**Entry points:** `POST /mcp` and `/mcp/<module>` (Streamable HTTP, JSON-RPC): `tools/list`, `tools/call`,
`resources/list|read`, `prompts/list|get`, `server/discover` (`crates/kernel/src/http/mcp.rs`,
`http/mcp/protocol.rs`, `http/mcp/transport.rs`; D-068, D-20261121-mcp-server). The manifest is
`docs/mcp-tools.json` (about 310 tools).

**Trust assumptions:**

- MCP is authenticated exactly like REST (`Authorization: Bearer` access token or API key), in a layer before the
  protocol code.
- A tool handler is a thin adapter: it calls the same service function as the REST handler, and passes the
  `Principal` whenever REST limits that permission by scope.

| STRIDE | Threat | Existing control | Residual / open |
| ------ | ------ | ---------------- | --------------- |
| S | Call tools without a credential or with a forged one | Same authenticator as REST, 401 before protocol handling (`http/mcp.rs`); 312 MCPAUTH cases per database passed | — |
| S | DNS rebinding against a local MCP client | `Origin` checked against the same `OriginGrant` as CORS; tenant-domain origins bound to their tenant (`http/mcp/transport.rs`) | — |
| T | A tool writes outside the caller's scope (branch, own records) | Gate `mcp-principal`: a tool whose permission REST checks by scope must use its `Principal` (`xtask/src/mcp_principal.rs`, D-20261121-security-lints); tools call service functions, not tables | The gate is textual: permissions passed through variables and named handler functions are not checked (documented limits in the rule) |
| T | Money or destructive actions from a stale session | Step-up applies to tool calls exactly as to REST (`security/access/step_up.rs`, D-20261119-step54-step-up; M-3 for automations) | — |
| R | Agent actions cannot be attributed | One request-log row per tool call (`mcp:<tool>`, no parameters) with the API key; changes go to the audit chain with actor and key (`ops/request_log.rs`) | — |
| I | `tools/list` reveals what the caller may not use | Lists, resources and prompts filtered by permission; a forbidden tool answers like a nonexistent one (`http/mcp.rs`) | — |
| I | List tools return more rows than REST | Same `limit` / `cursor` and `pagination` as REST (CLAUDE.md "Luật code") | 362 MCPIDOR cases per database: nothing of the other tenant returned |
| D | Huge `tools/list` payloads, expensive calls | One endpoint per module (`/mcp/<module>`); REST and MCP authenticate through the same per-API-key rate-limited authenticator (`http/mod.rs`, `rate_limit::ApiKeyLimited`) | — |
| E | A tool reaches what REST with the same permission cannot | Rule 8; `mcp-principal` gate; L-2 fixed (tool missing a REST check) | MCP tools are not yet linked to REST `operationId` one by one; the test team asked for it (P17, item with PT-11, PT-12, PT-18) |

## 3. Kernel

The kernel (`crates/kernel`) holds what every module relies on. The authentication module `auth` is a module, but
its guarantees are kernel-level for this model, so it is covered here.

**Assets:** signing keys and sealed secrets; accounts, passwords, second factors, sessions, API keys; the permission
catalog and role assignments; the organization tree; idempotency records; the outbox and the job queue;
configuration.

**Entry points:** `/api/v1/auth/*` (sign-in, refresh, logout, MFA, passkeys, sign-in links, reset, invitations,
OIDC, SCIM, API keys, roles, delegations, support grants); `/api/v1/kernel/*` (settings, tenants, domains, jobs,
records, tags…); background jobs and the event dispatcher; process configuration.

**Trust assumptions:**

- A business module checks permissions only through `Principal` (`security/access/mod.rs`) and never depends on
  `auth` (D-046).
- Jobs and event handlers receive their tenant from the stored row, not from a request.
- Configuration comes from the environment through `ConfigReader`; per-tenant settings go through the tenancy
  settings API.

| STRIDE | Threat | Existing control | Residual / open |
| ------ | ------ | ---------------- | --------------- |
| S | Forged access tokens (`alg` confusion, other tenant's `tid`) | Hand-written JWT accepting only `HS256`, signature checked in constant time, per-tenant derived key (`modules/auth/src/token.rs`, D-045, D-285); token checked against a live session; lifetime 900 s by default (`AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS`) | 47 TOKEN cases per database passed |
| S | Stolen refresh token | Rotation with families and reuse detection: reuse revokes the family (`modules/auth/src/session/`); only a hash is stored (`session/entity.rs` `token_hash`) | — |
| S | Password guessing | Argon2id (OWASP parameters, `modules/auth/src/password.rs`); throttling per email + address pair with a wider cap per email, counted in the database (`session/throttle.rs`, D-079, D-116, D-20261121-login-throttle-pair) | — |
| S | Second-factor bypass | TOTP codes single-use per step (`mfa/service.rs`), recovery codes; roles that require two-step sign-in apply in delegations and support access too (M-4); passkey-only accounts must use the passkey (M-8); passkeys via `webauthn_rp` with single-use ceremonies (D-077) | Passkeys and OIDC were not tested end to end in a browser (2026-11 "not covered") |
| S | SMS one-time codes guessed or flooded | Per-number, per-client-block and shared caps (PT-4, PT-7, D-20261125-otp-wrong-by-block) | The shared cap still lets a stranger lock a number out of SMS sign-in for a while (PT-7 follow-up, P17) |
| S | One-time tokens (reset, verify, sign-in link) reused or moved between tenants | 32 random bytes, SHA-256 stored, purpose- and tenant-scoped, consumed atomically (`security/tokens.rs`, D-117); link tokens carry the tenant (`data/tenancy/link_token.rs`, D-281) | — |
| S | API keys used from elsewhere | Key = scope ∩ owner's current rights, hashed with SHA-256 at rest, optional IP / CIDR fence, rotation (`modules/auth/src/api_key/`, `access.rs`); keys cannot create keys, SCIM tokens or sign-in providers (P-1, `auth.session_required`) | — |
| T | Tamper with sessions or roles to gain rights | "Only grant what you hold", also to oneself (PT-9, D-20261126-no-self-grant); SCIM changes judged by the token creator's rights (PT-1..3, D-20261125-scim-grant-cover) | Several grant-rule inconsistencies (scope vs. name comparison, 422 vs. 403, narrowing another person's role) are listed under P16 |
| T | Decide on an organization unit that is not in the tenant | `OrgTree::contains` / `ensure_exists` (PT-8, D-20261125-org-must-exist); closure table (`data/tenancy/org_tree.rs`) | Older rows pointing at unknown units need a clean-up command (P17) |
| T | Jobs or events processed twice or by two workers | Jobs claimed with `SKIP LOCKED` (PostgreSQL) or one atomic statement (SQLite) behind the `Claim` trait (`job/claim.rs`, D-009); events written in the business transaction and claimed with a conditional update (`event/outbox.rs`, `event/dispatcher.rs`, D-054); subscribers must tolerate redelivery | Multi-server CI test is a separate G13 item |
| T | Change configuration at runtime | Settings declared once with `Setting` and read through `ConfigReader`; no direct `std::env::var` (gate `env-access`); per-tenant secret settings write-only and sealed (`contracts/provider.rs`, `data/tenancy/settings.rs`) | — |
| R | Security actions without a trace | Events for sign-in, role changes, grants, delegations, support access, key changes go to the audit chain; support access needs a reason (`modules/auth/src/support.rs`) | The audit chain is unkeyed: see boundary 5 |
| I | Secrets at rest readable from a database dump | TOTP secrets, webhook secrets, provider keys and placement URLs sealed with AES-256-GCM bound to purpose, tenant and row (`security/seal.rs`, D-257); key rotation with `autoxide secrets reseal` | If `AUTOXIDE_SECRET_KEY` is unset, the root key is stored in the database (`kernel_secrets`) and travels with backups (`security/secret.rs`, D-113) |
| I | Personal data in logs | No bodies, tokens, query strings or raw IPs in logs; IPs hashed with a keyed HMAC by /24 or /64 where stored (`security/ip_hash.rs`, D-115); first admin's email not logged (I-4) | Field-level encryption of sensitive personal data is not implemented (G13 item) |
| D | Throttle tables or idempotency tables grown without bound | Rows expire and cleanup jobs delete them **(to verify for every throttle table)** | — |
| D | Outbox / job poll load | Idle passes cost one `EXISTS` per database (D-20261126-idle-polling); graceful stop between batches (PT-17) | — |
| E | A narrow principal reaches admin actions | Permissions named `<module>.<action>`, scopes per organization unit, `Principal::only` for automations (D-20261126-automation-owner-revoked) | `run_as_limit` compares permission names only, not scopes; a dead-lettered revocation event does not pause a rule (P16) |
| E | Platform staff read a tenant without consent | Entering a tenant needs a time-limited support grant with a reason; a tenant can turn support access off (`modules/auth/src/support.rs`, D-313) | — |

## 4. Modules

**Assets:** each module's tables and invariants (stock, money, publication state); the contracts between modules
(traits in `api`, events); the public Rust API used by modules outside the repository.

**Entry points:** a module's routes, tools, event subscriptions, jobs; `context.require::<dyn Trait>()`;
`Autoxide::new().module(…)` for external modules (`crates/app/src/lib.rs`).

**Trust assumptions:**

- Modules in this repository follow the boundary rules, enforced by `cargo xtask check` (D-031): a module exposes
  only `pub mod api` and `pub fn module(…)`, does not import another module's internals (`module-import`), does
  not query the database in handlers (`handler-database`), does not read the environment (`env-access`).
- A module owns its tables (`<module>_…`), with no foreign key across modules and no join into another module's
  tables (rules 1 and 4).
- **External modules are trusted code.** They are compiled into the same process, with the same database
  connections and keys (D-20261121-external-modules). The repository's gate rules do not run on code outside the
  repository.

| STRIDE | Threat | Existing control | Residual / open |
| ------ | ------ | ---------------- | --------------- |
| S | A module pretends to be another to publish events or provide a trait | Event names carry the module prefix and are declared through `Module::events`; traits are registered once at start (`module/registry/`) | Inside one process this is a code-review matter, not a runtime boundary |
| T | A module writes another module's tables | Table-name prefix per module and the ownership test in the gate ("luật sở hữu bảng"); `module-import` rule | Not enforced for external modules |
| T | Business invariants broken by races (overselling, double use of a coupon, double settlement) | Atomic bounded updates and conditional claims (review "Money"; M-2); concurrency tests on both databases (CLAUDE.md "Gate") | P17 follow-ups: price caps for very large values, currency checks in some purchase and POS paths |
| T | Event payloads trusted blindly by a subscriber | Subscribers re-read state from their own tables or the publisher's trait; must tolerate redelivery (D-054) | Events of modules with unit data must carry `org_id` everywhere (P16, D-20261125-event-org) |
| R | Changes made through a module are not traceable | `audit` subscribes to every event and appends to a hash chain (`modules/audit/src/lib.rs`, D-055, D-183) | — |
| I | Personal data left behind after an account is deleted or exported partially | Modules register their personal data for export and deletion (M-7; `contracts/privacy.rs`) | Each new module must register; no gate checks it **(to verify)** |
| D | A turned-off module still reachable | A module turned off serves no routes, tools, jobs or events (`module/selection.rs`, `module/dormant.rs`, [`docs/MODULES.md`](MODULES.md)) | — |
| E | Experimental modules weaken the core | Off by default; an official module never depends on an experimental one ([`docs/MODULES.md`](MODULES.md)) | Experimental modules got the same automated tests but less review |
| E | An external module bypasses the kernel's checks | None at runtime: Rust has no in-process sandbox | Installers must review external modules like any dependency; the [module guide](MODULE_GUIDE.md) asks authors to use `Principal`, `.scoped(&tenant)` and the `api` contracts |

## 5. Database

**Assets:** all data at rest; schema integrity; backups; the migration history; the audit chain.

**Entry points:** the server's connections (read pool and write connection per database); `autoxide migrate`,
`autoxide backup`, `autoxide restore`, `autoxide copy` (CLI on the server); scheduled backups to the file store; the
tenant export job.

**Trust assumptions:**

- Every feature runs on SQLite and PostgreSQL (CLAUDE.md "Luật số một"); a check that only holds on one database
  is a finding.
- Every tenant-level table has `tenant_id`, and every query on it filters through `.scoped(&tenant)`
  (`data/tenancy/scope.rs`, D-253) — also when the tenant has its own database.
- Only the operator can reach database files, credentials and backups.

| STRIDE | Threat | Existing control | Residual / open |
| ------ | ------ | ---------------- | --------------- |
| S | A request writes through the read connection | Read connections are read-only on both databases (SQLite `read_only`, PostgreSQL `default_transaction_read_only`) (`data/database.rs`, D-009) | — |
| T | A query forgets the tenant filter | `.scoped(&tenant)` and gate `tenant-scope` (`xtask/src/tenant_scope.rs`, D-20261121-security-lints; M-6 extended it to module macros); deliberate cross-tenant reads must carry a `// tenant-scope:` reason | The rule is textual: generic `E::find()`, queries passed to helpers and single-row `update` / `delete` are not seen (documented limits). 1,119 REST + 362 MCP cross-tenant cases per database found nothing |
| T | SQL injection through hand-written statements | `database::statement` with bound, typed values (D-20261121-raw-statement); raw SQL only where measured (CLAUDE.md "Luật dữ liệu") | — |
| T | Two processes migrate at once | Migration lock: PostgreSQL advisory lock, operating-system file lock for SQLite (`data/migration/lock.rs`); committed migrations are never edited | — |
| T | Undetected edits to audit rows | Audit rows form a SHA-256 chain; `autoxide doctor` reports the first edited, deleted or inserted row (D-183) | The chain is **not keyed and not anchored outside**: someone with write access to the database can rebuild it. A tamper-resistant log is a G13 item |
| T | Ledger entries edited outside the module | Append-only entries, reversals instead of edits, hash chain (`modules/ledger/src/lib.rs`, D-342) | Same limit as the audit chain |
| R | Operator actions on the database are not recorded | Out of scope (operator is trusted) | — |
| I | Backups readable by other users of the machine | Backup directory created `0700` (`data/backup.rs`, D-158); scheduled backups skip earlier backups | The server does not encrypt backup files itself; a backup holds password hashes, sealed secrets and, if `AUTOXIDE_SECRET_KEY` is not set, the root key. Encrypting before off-site copy is an operator step ([`docs/DEPLOY.md`](DEPLOY.md), "Encrypted off-site backups") |
| I | Tenant export leaks secrets | Export job omits the job queue, outbox and secret columns (hashes, secrets); download needs a signed-in request or a 24-hour signed link; platform staff only through a valid support grant (`data/tenancy/exports.rs`, D-328) | — |
| I | `autoxide copy` mixes data into a used database | Target must be new (no migrations before), one transaction, same schema version (`data/transfer.rs`, D-070) | — |
| D | SQLite write contention | One write connection, writes queue in the application (D-009); no `busy` error seen under 1,024 connections (2026-11 test) | Capacity guidance per database is a separate G13 item |
| D | Unbounded lists or deep offsets | Keyset pagination only (`fetch_page`, no `OFFSET`), index per sorted column; gate `api-unpaged-list` | Older lists still waiting in `xtask/api-shape-allow.txt` (P9) |
| E | Tenant code reaches control-plane tables | Control-plane tables (tenants, domains, placements, moves) live in the control database; tenant routes reach them only through platform-only services (`data/tenancy/admin.rs`, `require_platform`) | — |

## 6. Placement

**Assets:** which database and file store hold each tenant; placement connection URLs; domain → tenant mapping;
data residency promises; tenants during a move, split or merge.

**Entry points:** `autoxide tenant move` and the platform-only REST / MCP endpoints for placements, moves and
relocations (`data/tenancy/placements.rs`, `moves*`, `relocate*`; permissions `kernel.placements.read`,
`kernel.tenants.move`, `kernel.tenants.relocate`); domain endpoints of each tenant (`data/tenancy/domains*`);
sandboxes (`data/tenancy/sandbox*`).

**Trust assumptions:**

- Only accounts of the platform tenant can move, split, merge or erase tenants (`require_platform`); a staff member
  who entered a tenant through support access cannot.
- A tenant on its own database is still filtered by `tenant_id` (defence in depth).

| STRIDE | Threat | Existing control | Residual / open |
| ------ | ------ | ---------------- | --------------- |
| S | A tenant administrator triggers a move or relocation | `require_platform` + `kernel.tenants.move` / `relocate` (`data/tenancy/moves/http.rs`, `relocate/http.rs`) | — |
| S | A tenant keeps traffic for a domain it no longer controls | Daily DNS recheck; three misses → `failed`; DNS errors do not count as misses (`data/tenancy/domains.rs`) | Up to about three days of stale routing after a domain changes hands |
| T | Data lost or duplicated during a move | Dry run by default; preflight checks (schema version, running moves); the copy runs as a job through state phases; tables are copied by `tenant_id` with typed columns (`data/tenancy/moves/`, `data/transfer.rs`) | Moves under load and the dedicated-database placement were not covered by the 2026-11 test |
| T | A split or merge breaks references | Each module declares a relocation policy per table (`Module::relocation`, `data/tenancy/relocate/policy.rs`, D-20261118-step38-relocation) | A policy inferred from columns is a guess; the binary's test blocks undeclared tables (`crates/kernel/src/module.rs`), which covers modules in this repository only |
| I | Placement credentials disclosed | Connection URLs and own-storage settings sealed, bound to the placement id (`data/tenancy/registry.rs`) | — |
| I | Data leaves the agreed country | Per-tenant residency; moves to a placement in another or unknown country are refused (`data/tenancy/residency.rs`, D-360) | The country of a placement is declared by the operator, not measured |
| I | Sandbox copies personal data | A sandbox copies configuration only, stays on the source tenant's placement and expires (`data/tenancy/sandbox.rs`, D-232) | Completeness of "configuration only" per module **(to verify)** |
| D | Too many open pools | At most `AUTOXIDE_TENANCY_MAX_OPEN_DATABASES` pools, least recently used dropped (`data/tenancy/registry.rs`) | — |
| D | Cache staleness after suspension or domain removal | Invalidation at once on the server that changed it; others within `AUTOXIDE_TENANCY_CACHE_SECONDS` | Suspension and domain removal take up to one TTL on other servers |
| E | A tenant reaches another tenant's database | The registry opens a placement only for the tenant found from the credential, header or verified domain; no tenant-supplied connection strings | — |

## 7. External providers

**Assets:** money flows (payments, refunds, bank credits — the core never holds buyers' money, D-187); shipment state;
tax invoices; per-tenant provider credentials; outbound webhook secrets; the server's network position (SSRF).

**Entry points:**

- **Inbound:** `/api/v1/ledger/webhooks/{provider}/{hook}`, `/api/v1/ledger/bank-feeds/{provider}/{hook}`,
  `/api/v1/shipping/webhooks/{carrier_code}/{hook}/{token}` (and older forms); MoMo IPN, SePay bank feed, GHN status
  callbacks (`modules/momo`, `modules/sepay`, `modules/ghn`).
- **Outbound:** payment, carrier, SMS (`modules/esms`), e-invoice (`modules/einvoice_sepay`) and mail calls; outbound
  webhooks (`modules/webhooks`, Standard Webhooks signatures, D-081); OIDC discovery; tenant SMTP; web push
  (`WebPushSender` contract).
- **Addresses chosen by users:** webhook targets, OIDC issuers, SMTP hosts, push subscription endpoints, payment
  `return_url`, KYC `return_url`.

**Trust assumptions:**

- Every provider account belongs to a tenant and is configured by that tenant; the server never falls back to
  another tenant's account (`crates/kernel/src/contracts/provider.rs`, D-20261121-provider-accounts).
- Provider connectors in this repository are **experimental**: written from public documentation, not yet run
  against the live services (header of each connector's `lib.rs`).

| STRIDE | Threat | Existing control | Residual / open |
| ------ | ------ | ---------------- | --------------- |
| S | Forged payment, bank or carrier callbacks | Tenant found first from an unguessable per-tenant identifier in the URL, then the provider signature is checked with that tenant's secret, in constant time (`contracts/provider.rs`, `modules/momo/src/gateway.rs`, `modules/sepay/src/feed.rs`) | Real provider signatures were not tested (fakes only) |
| S | A callback from one seller's carrier account updates another seller's shipment | Updates only match shipments booked through the receiving carrier account (H-1, `modules/shipping/src/webhooks.rs`) | — |
| T | Amount, currency or account mismatch settles a payment | Amount and currency checked against the stored payment; bank credit must reach the payment's bank account (M-1); reconciliation item claimed before settlement (M-2); refunds bounded by amount captured (L-6); refund provider must match (I-1) | Foreign-currency payment after reconciliation goes to dead letter without an operator guide (P17) |
| T | Replayed callbacks | Deliveries deduplicated (review "Inbound webhooks") — per-connector mechanism **(to verify)** | — |
| R | Disputes about what a provider said | Provider calls logged without secrets, with metrics (`contracts/provider.rs` `send`); ledger entries append-only | — |
| I | Provider secrets leak | Secret settings sealed per tenant and row, write-only through the settings API (`contracts/provider.rs`, D-257) | — |
| I | Open redirect after payment or identity check | Payment `return_url` must be `https://` on a verified domain of the tenant or a listed origin (P-5, `ledger.return_url_not_allowed`) | KYC `return_url` has no allow-list yet (L-11, P16); no identity provider connector ships today |
| I | SSRF: the server calls internal addresses | Shared filter `crates/kernel/src/security/net.rs` (D-149, D-20261121-ssrf-targets): non-public IPs refused at save; names resolved through `PublicOnly` at connect time (DNS rebinding), redirects off, system proxy ignored (L-4); non-HTTP protocols resolve once and connect to that IP | 65 SSRF cases per database: nothing reached the local listener |
| I | Web push to internal addresses | Push subscriptions accept public hosts only; the `WebPushSender` contract requires public-IP connections (PT-10, `contracts/push.rs`) | No real web push sender ships in this repository |
| D | Outbound calls hang or return huge bodies | `http_client` with timeout and no redirects; response bodies capped by `AUTOXIDE_PROVIDER_MAX_RESPONSE_BYTES` (I-2); webhook bodies capped per route (`ledger` `webhook_max_bytes`) | — |
| D | SMS cost abuse (each SMS paid by the shop) | Per-number, per-client-block and per-hour limits (P-2, PT-4) | — |
| E | Provider calls inside a write transaction lock the database | Rule: no network call inside a write transaction (CLAUDE.md "Luật dữ liệu", `contracts/provider.rs`) | Enforced by review, not by a gate |
| E | Outbound webhooks leak events the receiver's owner may not see | Creating a webhook requires the event's stream permission; payloads signed (`modules/webhooks`) | — |

## 8. AI

**Assets:** tenant content sent to a model; the operator's model budget; integrity of what gets published; the
instructions given to the model.

**Entry points:** features that call `TextModel` or `Embedder` (`crates/kernel/src/contracts/ai.rs`, published by
`modules/ai`): SEO and alt-text suggestions, translations, product description drafts (`catalog`), support reply
drafts (`support/src/ai.rs`), review summaries (`orders/src/reviews.rs`), conversation drafts, search by meaning.
AI agents as API clients are covered by boundaries 1–3.

**Trust assumptions:**

- AI is off until the installer configures a provider ([`docs/AI.md`](AI.md)).
- Model output is untrusted data: it must match a JSON Schema and only becomes a **suggestion or draft**; a person
  (or an agent with the publish permission) decides (D-109).
- Rules for the model are in `instructions`; user data is only in `input` (`Prompt::new(…).input(…)`).

| STRIDE | Threat | Existing control | Residual / open |
| ------ | ------ | ---------------- | --------------- |
| S | Content an attacker controls (a review, a ticket, a product text) tells the model to do something else (prompt injection) | Instructions and input separated; output must match a JSON Schema (`ai.output_invalid`); the model has no tools and no credentials; results are drafts only (D-109) | Injection can still bias a draft; the human reviewer is the control. AI-generated entries carry `ai_generated` and publication records `ai_reviewed` ([`docs/AI.md`](AI.md)) |
| T | A draft overwrites a human edit | Translation saves only if nobody changed the target since the request (`content.translation_conflict`) | — |
| T | Model output carries unsafe markup or links | Translation keeps links, images and ids from the original; numbered tags must round-trip exactly | Sanitising of every AI draft before it is shown by a client **(to verify per feature)** |
| R | Who published AI-generated text | `ai_reviewed` with person and time on publication; audit chain | — |
| I | Sending personal or private data to a provider | Documented list of what is sent; private images only with `AUTOXIDE_MEDIA_AI_SEND_PRIVATE=true`; support history sent without email and phone; prompts, content and keys never logged ([`docs/AI.md`](AI.md), `modules/support/src/ai.rs`) | The provider is chosen by the installer; data processing agreements are theirs |
| D | Budget exhaustion | Daily token limit per tenant, from its plan (`ai.daily_tokens`) or `AUTOXIDE_AI_DAILY_TOKEN_LIMIT`, counted in the database across servers (`modules/ai/src/config.rs`, `ai_usage`); jobs cut off and retried; per-key rate limits | — |
| E | An AI agent publishes or deletes beyond its role | `_own` permissions limit an agent key to entries created with the same key; no archive / delete without the permission (`content.write_own`, `content.publish_own`) | — |

---

## Out of scope

- The operator: anyone with a shell, the environment, the database or the backups. Misconfigurations that
  [`SECURITY.md`](../SECURITY.md) lists (CORS `*`, public `/metrics`, rate limits at `0`, client-address header
  without a proxy, unset secrets).
- Volumetric denial of service; cheap single-request amplification **is** in scope.
- Administrators doing what administrators may do inside their own tenant.
- User interfaces and SDK consumers (separate repositories), the reverse proxy, TLS configuration, the operating
  system, the container runtime, DNS of tenants' domains.
- Real provider accounts (payments, bank, SMS, carrier, e-invoice, identity) and AI providers; their own security.
- Supply chain beyond what `cargo deny`, `gitleaks`, pinned actions and signed releases cover
  ([`docs/VERIFYING.md`](VERIFYING.md)).
- Legal compliance as such (see [`docs/COMPLIANCE.md`](COMPLIANCE.md)); this model covers its technical controls only.

## Known open items

Status lives in [`docs/ROADMAP.md`](ROADMAP.md), step 70, items **P16** (follow-ups of the November 2026 review) and
**P17** (fixes from the November 2026 test), and in the status table of
[`docs/security/pentest-2026-11/README.md`](security/pentest-2026-11/README.md#status-of-the-findings). At the time
of writing, the open classes are:

| Class | Boundary | Where tracked |
| ----- | -------- | ------------- |
| Open redirect class: identity verification `return_url` without an allow-list | 7 | P16 (L-11) |
| Grant-rule inconsistencies (permission name vs. scope; status codes; narrowing a role someone else holds; SCIM pending members, creator of a SCIM link) | 3 | P16 |
| Automation rules: owner's scope not captured; revocation event in dead letter does not pause a rule | 3 | P16 |
| Events of unit-level data without `org_id` in some modules | 4 | P16 |
| Cache eviction under random `Host` floods; WebSocket `Origin`; deterministic test of the domain-claim race and of a hung domain snapshot read | 1, 6 | P16 |
| Expensive public search (full-text index behind a trait) | 1 | P17 (PT-15) |
| Information exposure of low value: unused taxonomy terms, analytics pollution, newsletter confirmation cap | 1 | P17 (PT-11, PT-12, PT-18) |
| Very large prices or weights saturating instead of being refused; currency checks in some purchase / POS paths | 4, 7 | P17 |
| Records pointing at unknown organization units created before PT-8 | 3 | P17 |
| SMS sign-in shared cap can lock a number for a while | 3 | P17 (PT-7 follow-up) |
| `address-key` gate catches only one textual pattern | 1 | P17 |
| Tamper-resistant audit log (keyed or anchored chain); field-level encryption of sensitive personal data with managed and rotated keys | 3, 5 | G13 (this step) |

Only classes are named here. Details of an open issue are shared privately, through the channel in
[`SECURITY.md`](../SECURITY.md).

## How to use this for the external test

**Set-up.** Use `AUTOXIDE_TENANCY_MODE=multi`, at least two tenants plus the platform tenant, an organization tree
with unit-scoped staff, and both databases. The scripts of the November 2026 round
([`docs/security/pentest-2026-11/scripts/`](security/pentest-2026-11/scripts/)) build such a set-up and a case
matrix; reuse them as a baseline rather than repeating the generated groups. Point every provider at a local fake or
a sandbox account you own; test only your own installation.

**Suggested focus, most value first:**

1. **What the earlier rounds did not cover:** passkeys and OIDC end to end in a browser; payment, bank-feed and carrier
   callbacks with real provider signatures (sandbox accounts); tenant move, split and merge between databases under
   load; tenants on their own database; file and media uploads at their size limits; WebSocket beyond the per-user
   cap; `ai` with a real model.
2. **Grant and scope logic (boundary 3):** roles, memberships, delegations, support grants, SCIM, API key scope ∩
   owner, automations' `run_as`, step-up — look for any path where a principal ends up with more than its creator
   or holder. The P16 list names the areas known to be inconsistent.
3. **Tenant and branch isolation where the gate is blind (boundary 5):** code paths the `tenant-scope` and
   `mcp-principal` rules do not see (generic queries, helpers, named handler functions) — compare MCP tools with
   their REST endpoints one by one.
4. **Host-based tenant resolution and CORS (boundary 1):** verified domains, `?site=`, domain changes between
   tenants, cache TTLs across several servers, suspended tenants.
5. **Money paths (boundaries 4, 7):** currency and amount checks across every payment, refund, POS, stored-value
   and promotion path; races on both databases.
6. **AI drafts (boundary 8):** whether injected content in reviews, tickets or product text can produce a draft that
   a client would render unsafely or that misleads a reviewer.
7. **Each control marked (to verify)** below.

**Reporting.** Use the severity scale of the [November 2026 review](security/review-2026-11.md#severity) and the
private channel in [`SECURITY.md`](../SECURITY.md). Name the boundary and STRIDE category from this document.

## Controls to verify

Controls described by the design or documentation but not traced end to end in the code for this document:

1. Every throttle and idempotency table has an expiry and a cleanup job (boundary 3).
2. Every module that holds personal data registers it for export and deletion; nothing enforces it for new modules
   (boundary 4).
3. A sandbox copies configuration only, for every module (boundary 6).
4. Inbound provider callbacks: the replay / deduplication mechanism of each connector (boundary 7).
5. AI drafts: output sanitising for every feature that stores model text (boundary 8).

## Keeping this document current

Update this file in the same pull request as a change that adds a boundary, an entry point class (a new kind of
credential, a new inbound callback, a new outbound protocol) or a control listed here, and keep the Vietnamese
version in step.
