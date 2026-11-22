# Modules: what runs, and how to choose

[Tiếng Việt](MODULES.vi.md)

The Autoxide binary contains every module, but an installation only runs the modules it needs.
One setting chooses them: `AUTOXIDE_MODULES`.

## Three tiers

- **core** — the server does not run without them (sign-in, idempotent writes, and what those
  need). Always on; they cannot be turned off.
- **official** — on by default. Its API has no breaking change within one release line: a change
  that would break clients is made as a new endpoint, and the old one is declared deprecated with
  a removal date. A new release always reads the data of an older one (migrations run on their
  own). Security fixes reach every release line that is under long-term support.
- **experimental** — off by default; turn one on by naming it. It works and is tested on SQLite and
  PostgreSQL like the rest, but its API **may change between releases, breaking changes
  included**; each change is listed in the changelog. The system never deletes its data on its
  own, but moving that data to a new release may need a manual step, described in the upgrade
  notes. An experimental module can be removed if nobody uses it, or become official.

A module is in one tier as a whole; tiers do not split a module. An official module never depends
on an experimental one.

**Becoming official:** no open items left in the roadmap; real users; an API that did not have
to change over several releases in a row; and, for a connector to an outside provider, a test run
with a real account.

## The modules

`Needs` lists the modules a module cannot start without; they are turned on with it. A module also
*uses* other modules when they are on (for example `orders` takes shipping rates from `shipping`
and stock holds from `inventory`), and simply offers less when they are off.

<!-- modules:begin -->
| Module | Tier | Needs | What it does |
| --- | --- | --- | --- |
| `mail` | core | — | Outgoing mail for every module, queued and retried, over SMTP or to files in development. |
| `auth` | core | `mail` | User accounts, sign-in with passwords, passkeys and TOTP, access and refresh tokens, API keys, roles and permissions. |
| `idempotency` | core | — | Storage of `Idempotency-Key` results, so retried writes run once. |
| `ai` | official | — | Connection to AI models (text generation and embeddings) for every module; off by default. |
| `files` | official | — | File storage for every module, on disk or on S3-compatible storage. |
| `address` | official | — | Administrative units of Vietnam (provinces and communes after the 2025 merger) and address checks, shared by every tenant. |
| `organizations` | official | — | The organization tree inside each tenant (head office, regions, stores, sellers…), its unit types, life cycle and settings. |
| `audit` | official | — | Append-only audit log: who changed what, on which record, when, before and after. |
| `webhooks` | official | — | Signed webhooks (Standard Webhooks) that send module events to other systems, with retries and delivery history. |
| `tax` | official | — | Tax engine: tax groups and rates as dated data, tax profiles per organization unit, withholding and invoice requirements. |
| `ledger` | official | — | Double-entry ledger per organization unit: accounts, append-only entries, balances, holds, payouts and payments through licensed partners. |
| `invoices` | official | `tax`, `ledger` | Accounting documents and e-invoices: drafts, numbered issue, adjustment, replacement and cancellation. |
| `compliance` | official | — | Vietnamese compliance basics: identity checks, legal documents and acceptances, website registration, personal data requests and incidents. |
| `notifications` | official | — | Notifications to users by email, SMS, Zalo ZNS, web push, mobile push and an in-app inbox, with templates and preferences. |
| `plans` | official | — | Plans that sell modules, features and limits per tenant, with add-ons, overrides, subscriptions and usage. |
| `media` | official | `ai`, `files` | Media library: images, video, audio and documents in nested folders, with tags, alt text and resized copies. |
| `content` | official | `ai`, `media` | Posts and pages of a website in several languages: drafts, revisions, scheduling, categories, tags, authors and redirects. |
| `forms` | official | `mail`, `files` | Forms with typed fields, public submissions behind spam protection, an inbox, CSV export and retention. |
| `menus` | official | `media` | Website menus (top bar, mega menu, footer) whose items point to addresses or to records of any module. |
| `catalog` | official | — | Products, variants, category tree, price lists with history and price floors: the product catalog every commerce module reads. |
| `inventory` | official | `catalog` | Stock per variant and warehouse with an append-only movement ledger, reservations that never oversell, transfers, stock counts, lots, serials and purchasing. |
| `shipping` | official | — | Shipping: buyer address books, shipping rates set by each shop (zones by province, weight tiers, free shipping, store pickup), carriers plugged in as connectors, shipments with parcels and tracking, and cash-on-delivery reconciliation. |
| `promotions` | official | `catalog` | Promotions as rules: automatic discounts, coupons and vouchers, flash sales, tiers, bundles and gifts, stacking in three classes, usage holds for orders and legal limits as dated data. |
| `orders` | official | `catalog` | Orders: carts for buyers and guests, checkout with time-limited holds of stock and promotions, orders with sequential numbers and a lifecycle, payments through licensed partners, returns and refunds, electronic invoices, point of sale, product reviews. |
| `customers` | official | — | Customer profiles keyed by buyer account or phone number, purchase history from orders, masked contact details with audited reveal, and manual or rule-based customer groups for price lists and promotions. |
| `sepay` | experimental | — | Confirms VietQR bank transfers from SePay's bank transaction notifications, with one SePay account per tenant (experimental: written from the provider's public documentation; not yet verified against the live service). |
| `einvoice_sepay` | experimental | — | Sends e-invoices to the Vietnamese tax authority through SePay eInvoice, with one SePay eInvoice account per tenant (experimental: written from the provider's public documentation; not yet verified against the live service). |
| `esms` | experimental | — | SMS through eSMS (Vietnam): one-time codes and customer-care messages under each tenant's own eSMS account and brandname. Written from the provider's public documentation; not yet verified against the live service. |
| `bookings` | experimental | — | Bookable services with staff, rooms and equipment, weekly schedules, public availability, bookings that never overlap, deposits through a licensed partner, cancellation policies and reminders. |
| `newsletter` | experimental | `mail` | Newsletter lists, double opt-in subscribers, one-click unsubscribe and campaigns sent in rate-limited batches. |
| `comments` | experimental | `mail` | Comments and questions on any record (posts, products…), with threads, moderation, reports, bans and answers. |
| `support` | experimental | — | Customer support tickets: messages, internal notes, guest access by ticket token, staff scope by organization unit, and hand-over to consumer complaints. |
| `conversations` | experimental | — | Customer chat with a shop or a marketplace store: shared inbox, assignment, read cursors, real-time events, notices for people offline, hand-over to support tickets and automatic answers from approved replies. |
| `analytics` | experimental | — | Cookieless website analytics kept in your own database: pages, sources, UTM campaigns, most viewed objects and funnels; per-person tracking only with consent. |
| `reporting` | experimental | — | Reports across a customer tree: figures from every tenant (also on other databases), summed per day, tenant, unit and currency. |
| `automations` | experimental | — | Automation rules: when an event, a schedule, a manual call or an unusual figure happens and the conditions hold, run MCP tools with the permissions of the rule's author. |
<!-- modules:end -->

`GET /api/v1/kernel/modules` lists the modules that run on a server, with `stability` for each.
In the OpenAPI document every operation of an experimental module carries
`"x-stability": "experimental"` (an operation without it belongs to an official module), and
`docs/mcp-tools.json` has `stability` for every tool; both also say so in the description.

## Choosing: `AUTOXIDE_MODULES`

A comma-separated list. Each entry adds to the set; core modules are always in it.

| Entry      | Meaning                                                    |
| ---------- | ---------------------------------------------------------- |
| `core`     | Only the core modules.                                     |
| `official` | Every core and official module. **The default.**           |
| `all`      | Every module, experimental ones included.                  |
| `name`     | That module, and the modules it needs.                     |
| `-name`    | Leave that module out of the set chosen by the other entries. |

Examples:

```sh
# The default, written out.
AUTOXIDE_MODULES=official

# Official modules plus two experimental ones.
AUTOXIDE_MODULES=official,bookings,support

# A website backend: sign-in, pages and posts, media, forms, menus. `media`, `files`, `ai`
# and `mail` come along because `content`, `forms` and `menus` need them.
AUTOXIDE_MODULES=content,forms,menus

# Everything official except plans and the commerce modules that are not needed here.
AUTOXIDE_MODULES=official,-plans,-customers,-orders
```

Mistakes are reported at startup, by every command, with what to write instead:

- a name that is not a module lists the valid names;
- `-name` for a module that an enabled module needs names the modules to leave out too
  (`official,-catalog` answers: leave out `-inventory,-orders,-promotions` as well, or remove
  `-catalog`);
- `-auth` or another core module is refused.

`autoxide doctor` prints the modules that run and those that are turned off (the first line of
`autoxide serve`'s log says the same), and `autoxide config` shows the value in use.

Settings of a module that is turned off are not read: a leftover `AUTOXIDE_NEWSLETTER_…` line in
`.env` is ignored. A task in `AUTOXIDE_STARTUP_TASKS` whose module is off is skipped with a warning
in the log; a task name that does not exist in a module that is on is still an error.

## What "turned off" means

A module that is turned off:

- serves no routes: its paths answer `404`;
- is absent from the OpenAPI document served at `/api/v1/openapi.json`, from `/llms.txt` and
  from `GET /api/v1/kernel/modules`;
- offers no MCP tools, resources or prompts;
- runs no jobs and no schedules, and receives no events;
- has no migrations applied: on a new database its tables are never created.

The files `docs/openapi.json` and `docs/mcp-tools.json` in the repository, and the SDKs built from
them, always describe **every** module. `autoxide openapi` and `autoxide mcp-manifest` print that
full contract whatever `AUTOXIDE_MODULES` says. A client can ask a server which modules it runs
through `GET /api/v1/kernel/modules`.

### Data of a module that is turned off

Turning a module off **never deletes anything**. If the database already has its tables, they
stay exactly as they are, and turning the module on again continues from there (migrations added
in the meantime are applied by `autoxide migrate up` as usual).

While it is off, nothing maintains that data:

- personal-data exports and erasures, retention clean-up and tenant deletion do not reach it;
- tenants created in the meantime do not get the module's starting data;
- events other modules publish in the meantime are not delivered to it later.

So that data is not lost by accident:

- `autoxide serve` logs a warning at startup, and `autoxide doctor` reports
  `modules_turned_off`, when the database holds rows of a module that is off — with the exact
  `AUTOXIDE_MODULES=…` line that turns it back on. (When you set `AUTOXIDE_MODULES` yourself the
  message is informational: you chose.)
- `autoxide copy`, `autoxide backup create` and `autoxide backup restore` refuse to run while
  such data exists, because they only handle the modules that are on. Run them with the module
  turned on, or with `AUTOXIDE_MODULES=all`.
- Moving a tenant to another database, and splitting or merging tenants, are refused
  (`409 tenant_move_modules_off`, also listed by the dry run) while the tenant has rows of a module
  that is off: only the data of modules that are on is moved.

The check for such data looks at the main database and at the database of every placement.

To remove the data of a module for good: turn the module on, run
`autoxide migrate down <module> --steps 1000` (this drops its tables — back up first), then turn it
off.

## Upgrading from 0.17 or earlier

Up to 0.17 every module ran. From this release, with `AUTOXIDE_MODULES` not set, the eight
experimental modules (`bookings`, `newsletter`, `comments`, `support`, `conversations`,
`analytics`, `reporting`, `automations`) are **off**.

1. Before updating, decide which of them you use. If unsure, update and run `autoxide doctor`:
   `modules_turned_off` lists the ones whose tables hold rows, and gives the line to add.
2. Add that line to `.env` (or the environment of the container), for example
   `AUTOXIDE_MODULES=official,newsletter,comments`, and restart. To keep exactly the behavior of
   0.17, use `AUTOXIDE_MODULES=all`.
3. Roles keep the permissions of modules that are off, plans keep their features, and stored data
   stays; they take effect again when the module is turned on.

Nothing is deleted in either case, and the choice can be changed at any time.

## For module authors

A module declares its tier and what it needs in its `Module` implementation; all three have
defaults, so a module that declares nothing is official, optional and independent:

```rust
fn stability(&self) -> Stability { Stability::Experimental }
fn essential(&self) -> bool { false }
fn depends_on(&self) -> Vec<&'static str> { vec!["mail", "files"] }
```

List in `depends_on` every module whose contract you take with `context.require::<…>()?` in
`start`. Contracts you take with `.ok()` are optional and stay out of it. A test in the binary
starts each module with only the modules it declares, so a missing name fails the gate, not an
installation. See [MODULE_GUIDE.md](MODULE_GUIDE.md) for the rest of the contract.
