# RC trial, round 1 (AI agents playing users)

**English** · [Tiếng Việt](README.vi.md)

**Important:** the testers were AI agents playing a role, not people. Round 1 ran on `main` at version 0.19.0 (before
`1.0.0-rc.1`), as an early check (the project owner asked AI agents to play users before outside testers, in a few rounds). Every tester saw only public documents (README,
`docs/guides`, `docs/*.md`, OpenAPI, `examples/`), never the source of `crates/` or `modules/` (the module-author role
could also read `modules/<name>/src/api.rs`). Subagents: the newcomer role ran on a small model (to imitate a less
experienced person), the other four on a mid-size model. No real secret and no real provider or AI model was used.

Environment limits that shaped the results (not product bugs): 4 CPU cores, no Docker daemon (the Docker quickstart was
**not tried**), so every role used the binary, with SQLite or a local PostgreSQL 16. The binary was built once up front
(`cargo build -p autoxide`, debug, **10 min 06 s** on a cold cache); roles timed themselves from "binary exists". Timings
below come from the agents' own clocks and are approximate where stated.

## Numbers for G9 (agent playing a role, not a person)

| Measure | Result | Reading |
| --- | --- | --- |
| Minutes to the first request that returns data (binary already built, `.env` written) | **under 1 minute** (36 s from migrate to a data request) | Does **not** count reading the docs, writing `.env`, or building. Building from source took 10 min 06 s (debug, cold). Quickstart with Docker not measured. G9's "under 5 minutes from download" cannot be judged until a prebuilt binary or a working image path is measured. |
| Minutes to the first module (generate, build, run an endpoint) | **about 33 min**, of which 32 min 08 s was compiling the core into the module's own target directory | The build was slowed by a stray second build on the same 4-core machine, so a clean run is faster but not measured. Generate: 23 s. From a finished binary to a working endpoint: about 1 minute. G9's "under 30 minutes" is **at risk** on a cold build unless the guide offers a prebuilt option. |
| Shop owner, product to e-invoice | about 15 min (estimate) | About 6.5 of those minutes went to discovering steps missing from the guide (promotions, sandbox, simulated payment). |
| Operator, PostgreSQL from scratch to restored backup | 4 min 16 s | migrate 7 s, backup 4 s, restore 14 s, graceful stop 2 s. |
| AI-agent tasks (MCP) | 2.5 of 3 multi-step tasks done | The third (customer, product, draft order) cannot be done with the MCP tools alone: no tool for customers, orders or price lists. |

## Role 1 — Newcomer / AI-assisted coding (small model)

Goal: README to first request, then "ask an AI" to add a small feature.
Result: part 1 reached; part 2 **not reached** (the module build did not finish in 26 minutes: the machine was loaded by
other roles, see role 2).

| # | Level | Where | What happened | Suggestion |
| --- | --- | --- | --- | --- |
| 1 | annoying | README "Getting started", "Configuration" | Every command is `cargo run -p autoxide -- …`; a person without Docker or Rust does not know where to get a binary. `docs/DEPLOY.md` section B explains it but README does not point there. | Add a "no Docker" paragraph: download a release binary, or `cargo build --release`, and link to DEPLOY.md B. |
| 2 | annoying | README "Configuration" | Does not say `.env` needs two secrets (`AUTOXIDE_AUTH_TOKEN_SECRET`, `AUTOXIDE_SECRET_KEY`); `cp .env.example .env` leaves them commented. | Put the two `openssl` lines at that step. |
| 3 | annoying | README "Configuration" | No example of a GET that returns data; the tester guessed `/api/v1/address/provinces`. | Add one, and point to `docs/openapi.json`. |
| 4 | small | any CLI command | SMTP / files WARN lines print before every command output (even `task`, `user create`) and look like errors. | Show them only where mail or files are used. |
| 5 | small | writing-a-module.md §1, §5 | No estimate of the first build of an external module (the whole core, own target directory). | State the time; suggest a shared `CARGO_TARGET_DIR`. |
| 6 | small | README "Try it with Docker" | Mentions the Releases page without a link. | Add the link. |

Real bugs: none found in part 1. Not checked: external module, PostgreSQL.

## Role 2 — Module author outside the repository

Goal: follow `docs/guides/writing-a-module.md`, make a small module in a directory outside the repo, run `check-module`,
test on SQLite and PostgreSQL. Result: **reached.** The module `links` was generated with
`cargo xtask new-module --external … links link --core-path …` (the guide's way); it is a renamed copy of the sample,
the tester did not write new business logic. `check-module`: no violations. Tests: 8 + 4 pass on SQLite and on
PostgreSQL (port 15432). `POST` / `GET /api/v1/links` work, a duplicate gives 409 `links.url_taken`.

Time: generate 0:23; first server build 32:08 (compile wait, slowed by a stray build); binary to a working endpoint about
1:00; `check-module` 1 s when the machine is free; SQLite tests 6:41 (rebuild after a machine restart), PostgreSQL 10 s.

| # | Level | Where | What happened | Suggestion |
| --- | --- | --- | --- | --- |
| 1 | annoying | guide §4 "Run it" | "The first build takes a while" with no figure (about 30 min debug on 4 cores). | State it; suggest `--release` or a cache. |
| 2 | annoying | guide §4 | Only `http://127.0.0.1:8080` is shown; no variable to change the port. The tester guessed `AUTOXIDE_PORT` / `AUTOXIDE_BIND` (silently ignored; see UT-8). | Add "change the port: `AUTOXIDE_HTTP_ADDRESS`". |
| 3 | small | `migrate up` | Prints a line for each of 395 migrations, then a long backup job log. | Summary by default, detail with `-v`. |
| 4 | small | guide §5–6 | Suggests Docker for PostgreSQL; no hint for a local server without Docker. | One line for local PostgreSQL. |
| 5 | small | guide §6 vs `new-module` output | Two different ways to call `check-module` (`--manifest-path …/xtask/Cargo.toml` vs `cargo xtask`). | Use one. |

Real bugs: none.

## Role 3 — Connecting an AI agent (MCP)

Goal: connect over MCP with an API key, list tools, do three multi-step jobs, note tools whose description is not enough.
Result: **partly reached** (about 4 min, no build). Done: content (draft, search, update, revisions, review request);
catalog + draft invoice (tax line computed correctly); pagination and filters (three pages, 3+3+1, correct). Not done:
customer, product, draft order (no tools for it; see UT-5).

| # | Level | Where | What happened | Suggestion |
| --- | --- | --- | --- | --- |
| 1 | annoying | `.env` parsing | A password with spaces needs double quotes; the error names a line index, not the variable. | Comment in `.env.example`; name the variable in the error. |
| 2 | annoying | `docs/guides/mcp.md` §6 | Does not say `migrate up` must run before `serve` when starting the plain binary (388 migrations, about 20 s). | One line. |
| 3 | annoying | mcp.md §3 | How to make an MCP write idempotent is not explained: the `Idempotency-Key` header works on `tools/call`, some tools use a `reference` field instead; two different errors (`idempotency_key_reused`, `reference_reused`). | Document both and which tool uses which. |
| 4 | annoying | `content.*` `version` | The value is a quoted ETag string (`"\"1795…\""`); sending it without the quotes gives `precondition_failed`. The description does not say. | Accept both forms or say "send exactly as returned". |
| 5 | annoying | `precondition_failed` over MCP | The message tells you to send `If-Match` (a REST header). | Message for MCP: "read again and pass the new `version`". |
| 6 | blocking (part) | `catalog.add_prices`, `resolve_price` | Need `price_list_id` but no tool lists or creates price lists; none exist on a new install. No tool creates customers or orders (as mcp.md says). mcp.md §6's "draft an invoice at the list price" row does not run on a fresh install. | Add `catalog.list_price_lists`; say what to do first over REST. |
| 7 | small | `tools/list` | A key with only content permissions still sees 49 tools, including `auth.create_api_key` / `auth.rotate_api_key`, which always fail for an API key. | Hide them for API-key principals. |
| 8 | small | missing permission | A hidden tool answers "Unknown tool", same as a tool that does not exist; mcp.md does not say so. | Document it. |
| 9 | small | tool schemas | Many parameters have no description or enum (`catalog.list_products` `readiness`, `category_id`, `kind`; `update_product`; `upsert_variant` `create`/`update` typed null; `add_prices`; `invoices.create_draft` `buyer_*`; `resolve_price`; variant size and `meta_*` fields). | A gate that requires a description on every tool parameter. |
| 10 | small | `content.create_draft` | `locale` empty gives `en` even for a Vietnamese title; the default is not stated. | State the default. |
| 11 | small | `invoices.create_draft` | A wrong kind, currency and unit price together report only one error (the project rule is one pass with all errors). | Collect them. |
| 12 | small | cursors | A cursor is accepted with a different filter. | Bind the filter to the cursor. |
| 13 | small | `Idempotency-Key` reuse with another body | Returns a raw HTTP 422, not a tool error (`isError`); an MCP client sees an HTTP-level failure. | Wrap it as a tool error. |
| 14 | small | rate limit | Docs say 120/min/key; after a 62 s pause only 32 of 125 requests passed, then 429; the message says "address", not key; a revoked key used 65 times also gave 429, not 401. | Reconcile docs and behaviour. |

Real bugs:

- **An authorization scope problem in API-key management over MCP** (reported privately; details withheld until it is
  fixed and then published).
- `catalog.create_product` accepts `"tax_category":"nope"` and stores it (the description says it must be in
  `GET /api/v1/tax/categories`).
- `catalog.list_products readiness:"x"`, `audit.list_entries module:"nope"` or `subject:"bad"` return an empty list
  instead of an error (a wrong `status` does report one): inconsistent.
- Audit entries of an agent show `actor_kind:"user"` with `actor_api_key_id`, no key name; mcp.md says "under the key's name".

## Role 4 — Operator

Goal: set up with PostgreSQL 16 (no Docker daemon), migrate, doctor, metrics, backup, restore, graceful stop, upgrade.
Result: **reached** (4 min 16 s). All documented commands that could run did what the docs say. Restore: 460 tables, row
counts match (apart from rows the tester added by logging in), the old admin password works, 3355 address units and 8 tax
rates are intact. Not tried (no Docker, out of scope): `docker compose` / `docker run`, `gh attestation verify`,
systemd, SQLite→PostgreSQL `copy`, multi-tenant, OTLP, encrypted backup / PITR.

Upgrade: no earlier release to upgrade from. Simulated the documented module on/off path: `serve` warns
`modules_turned_off`, `doctor` warns, `autoxide modules` says "1 table(s) with rows", `backup` refuses with instructions;
`migrate down customers` then `up` is clean.

| # | Level | Where | What happened | Suggestion |
| --- | --- | --- | --- | --- |
| 1 | annoying | DEPLOY.md §B | No "binary + PostgreSQL from scratch" section; §B does not name `AUTOXIDE_HTTP_ADDRESS` (the tester tried `AUTOXIDE_HTTP_PORT`). | Add a short "B+" section. |
| 2 | small | `config`, `serve` | A misspelled `AUTOXIDE_*` variable is ignored; `config` says "valid"; the server listens on the default 8080. | Warn about unknown `AUTOXIDE_*` names. |
| 3 | small | `migrate down … --steps 0` | Prints "no applied migrations" for a module that has 13. | Reject `--steps 0` clearly. |
| 4 | small | `backup`, `restore` | 434 INFO "table copied" lines hide the result. | Print a summary only. |
| 5 | small | log while draining | Each `/health/ready` 503 `shutting_down` is logged at ERROR; `tenant_id=` repeats twice on request log lines. | Lower the level; drop the duplicate. |
| 6 | small | unreachable PostgreSQL | Wrong port: "pool timed out" after 10 s, no "connection refused"; some errors end with an odd "at line 331". | Show the root cause. |
| 7 | small | error text and CHANGELOG (~lines 396–397) | Say `autoxide backup create`; the real command is `autoxide backup <directory>`. | Fix the text. |
| 8 | small | DEPLOY.md "Settings worth knowing" | `AUTOXIDE_AUTH_TOKEN_SECRET` is "required in production", but without it the server runs with a random key and a WARN. | Say "recommended". |

Real bugs: none that give a wrong result (only #3, a wrong message). Good: error messages name the command to run,
`doctor` hints each fix, start-up in 1.1 s, `/metrics` on its own port, drain as documented.

## Role 5 — Shop owner using the API (curl and the TypeScript SDK)

Goal: customer, product, stock, promotion, order, simulated payment, draft e-invoice. Result: **partly reached.** The
whole sale flow ran with curl and with the SDK; money checked by hand (300,000; 480,000 with tax 43,636; 210,000 with tax
19,091), integer strings plus `VND`; four buyers in parallel for the last item gave exactly one order; partial refund,
cancel of a paid order and refund replays did not double. About 15 min (estimate); about 6.5 min of it was looking for
steps the guide lacks. The SDK runs directly with Node 22, no `npm install`.

| # | Level | Where | What happened | Suggestion |
| --- | --- | --- | --- | --- |
| 1 | blocking (part) | shop.md, sandbox.md, README | Only real-provider instructions. The sandbox says "payments succeed", but a `momo` payment stays `pending`; there is no official way to complete a simulated payment (the tester signed a MoMo notification with made-up keys). | A documented way to complete a simulated payment, or let the sandbox succeed by itself. |
| 2 | annoying | shop.md "What you need" | No path to try without provider accounts: the sandbox needs `AUTOXIDE_TENANCY_MODE=multi` and `autoxide tenant create`, not in the quickstart. | A "try it without real accounts" section. |
| 3 | annoying | sandbox.md | A new sandbox is empty (no products, prices, stock, shipping, tax profile) with no guide to build a shop in it; uses `uuidgen` (missing here); does not say `Autoxide-Tenant: <slug>` is needed on public routes too. | Say what is not copied; use `openssl rand -hex 16`. |
| 4 | annoying | shop.md | Missing steps: manual shipping (zone, method, rates); tax profile (`PUT /tax/profiles/{org}`: without it tax is 0, silently); invoice series (without it no invoice is issued, only a log line `invoices.series_not_found`); `jq` is required; `business_lines` values; `address` is an object. | Add the steps. |
| 5 | annoying | PROMOTIONS.md "Turning on" | Does not show how to create a code (`POST /promotions/{id}/codes` needs `code` + `purpose:"redeem"`), where `promotions.notice_province` lives (a unit setting: `PUT /organizations/units/{id}/settings/…`), or how to record the province notice (`PATCH …/notices/province:01`). | A complete curl sequence. |
| 6 | annoying | PROMOTIONS.md | A promotion is refused with `promotions.reason.no_reference_price` if the variant's price starts after the promotion's `starts_at`; so a shop that just set its prices cannot discount today. Not documented; the reason does not say how to fix it. | Document; improve the reason. |
| 7 | annoying | providers/momo.md | No signing formula or meaning of `orderId` for a local notification test. | Add a test example. |
| 8 | annoying | shop.md steps 4–5 | A guest does not get an `Order-Token` when ordering; it only comes from the SMS lookup (`lookup`, then `confirm`, field `access_token`). The guide says to send `Order-Token` without saying where it comes from. | Describe the flow. |
| 9 | small | shop.md | A cart lists `payment_methods` without `cod` / `wallet`, yet both are accepted. | Make the lists match. |
| 10 | small | checkout | `bank_transfer` without a bank account: `ledger.bank_account_not_found` does not say to create one (`POST /ledger/bank-accounts`). | Clearer message. |
| 11 | small | sandbox.md | Says e-invoice numbers start with `SANDBOX-`; the `number` is `1`, only `provider_ref` / `lookup_code` / `authority_code` do. | Fix the sentence. |
| 12 | small | `GET /promotions/notice-rules` | A `note` contains an internal decision id "(D-225)". | Remove it from the data pack. |
| 13 | small | sdk/typescript README | Imports `@autoxide/sdk`, but copying `src` does not create that name; no tenant-header example. | Relative import; show a `fetch` wrapper. |
| 14 | small | `…/issue` | With two series of the same kind the call fails with `series_id: is required`; the automatic issue at delivery only logs `validation_failed` without the reason. | Message names the fix (PATCH), log names the reason. |

Real bugs:

- **L1 (most serious): stock `on_hand` never goes down after delivery.** Sandbox tenant, one variant with 10 in stock, order
  2, pay (notification), create a shipment, `POST /shipping/shipments/{id}/status` through `ready_to_ship`,
  `picked_up`, `in_transit`, `delivered` (or `POST /orders/{id}/transitions` to `preparing`, `shipping`, `delivered`).
  Seen after waiting 20 s: order `delivered`, but `GET /inventory/levels` shows `on_hand 10, available 8, reserved 2`;
  `GET /inventory/movements` has only the `+10` adjustment, no `sale`; the reservation stays `confirmed`; the order's
  `fulfillment_status` stays `unfulfilled`. Expected (INVENTORY.md, Reservations: "fulfil — writes `sale` lines"):
  `sale −2`, `on_hand 8`, `reserved 0`, `fulfilled`. `available` is right, so no overselling, but the ledger of stock is wrong.
- L2: `payment_actions[].payment_id` from checkout (and from `POST /orders/{id}/payment-link`) is not the ledger payment id:
  `GET /ledger/payments/{id}` returns 404 `ledger.payment_not_found`; the real id appears only in `GET /ledger/payments`
  (filter `source_id`).
- L3 (small): repeating checkout, or cancelling a guest order, with the same `Idempotency-Key` returns 200 (first call 201)
  without the `Idempotent-Replayed: true` header, unlike retries.md and the OpenAPI text.
- L4 (small): `POST /orders/{id}/payment-link` after a failed first payment returns the first (failed) `payment_id` and
  `status_token`; that token stays `pending` while the ledger has a new payment with another id.
- L5 (small): a MoMo notification with the wrong amount sets `needs_review` (as documented); a correct notification sent
  afterwards does not bring it to `succeeded` and the order stays `pending_payment`; nothing says how an operator clears it.

## Summary by priority

1. **Blocking for 1.0:** UT-1 (stock never decreases on delivery), UT-2 (API-key scope problem over MCP, details withheld).
2. **Annoying, fix before the RC feedback window:** UT-3 (fastest path without Docker: binary, secrets, `migrate up`,
   port, PostgreSQL), UT-4 (no way to try a sale without real providers; shop guide missing steps), UT-5 (MCP tools:
   missing parameter descriptions, price lists / customers, validation of values), UT-6 (payment ids, idempotency
   headers, payment-link replay), UT-7 (first build time of a module outside the repo).
3. **Small:** UT-8 (messages and logs for operators), UT-9 (rate-limit docs vs behaviour, internal id in public data).

Note: the first-build time of a module (about 30 min) puts G9's "first module under 30 minutes" at risk.

Housekeeping: a second build left running by role 1 competed with role 2 for CPU for most of its wait; it could not be
stopped from this session. Files left in `/tmp` of the trial machine only; nothing was changed in the product code. Role 4
and role 2 left a local PostgreSQL running (ports 15434 and 15432).
