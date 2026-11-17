# Changelog

Notable changes to Autoxide. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html): while the version is `0.x`, a
minor release (`0.2.0`) may contain breaking changes, which are always listed under
**Changed** or **Removed**. The HTTP API itself never breaks within `/api/v1`: replaced
endpoints are marked deprecated with a sunset date first.

## [Unreleased]

### Added

- Inbox per person and setup checklist (step 51): `POST/DELETE /api/v1/me/work/{key}/snooze` hides a piece of work
  from your inbox until a time; `POST /api/v1/me/work/{key}/assign` assigns it to a user who can see its source
  (`Authenticator::principal_of`, default `None`); `GET /api/v1/kernel/work` shows `assignee_id`, filters
  `assigned=me|nobody` and hides snoozed work unless `include_snoozed=true`. Modules declare setup steps computed
  from their data (`StartContext::setup_step`); `GET /api/v1/kernel/setup` lists those that fit the tenant's
  business model with progress. Content: "publish the first page".
- Advanced search and open by code (step 51): `GET /api/v1/kernel/find?q=` understands `is:<type>`, `field:value`,
  `>`, `<`, `>=`, `<=`, `from..to`, `-` to exclude and quoted phrases (`SearchQuery`), and `scope=node|subtree|accessible`;
  parts no searched type understands come back in `ignored`. Posts and pages understand `status`, `kind`, `locale`,
  `updated` and `created`. `GET /api/v1/kernel/resolve?code=` opens the first record a module recognises by its code
  (`StartContext::code_resolver`; posts and pages by slug or address), with the same 404 for "none" and "not yours".
- Bulk actions (step 51): modules declare actions on a list (`StartContext::bulk_action`, `bulk` module of the
  kernel); `POST /api/v1/kernel/bulk/{resource}/{action}` takes `ids` (up to 500) or a `filter` with
  `expected_count` (409 `kernel.bulk_count_changed` when the matches changed) and an `Idempotency-Key`, and runs
  the action in a background job with the requester's permissions; the job's result lists what happened to each
  record; `dry_run` lists the records it would act on without doing anything. Posts and pages can be archived,
  comments approved, rejected, marked as spam or deleted, and newsletter subscribers blocked, unblocked or deleted
  in bulk.
- Step 37, mandatory page templates in `content` (Vietnam; every text says it needs review by a legal professional):
  owner information, personal data protection, returns, shipping, payment, inspection, complaints, terms of use,
  general conditions, and for a marketplace its regulations, the seller agreement and seller information, in English
  and Vietnamese (`modules/content/templates/legal/`). `GET /api/v1/content/templates`,
  `POST /api/v1/content/templates/{key}/drafts` make a draft page filled with the unit's legal profile (from
  `compliance` through the new `LegalProfiles` contract) and the given `variables`, through the kernel template engine,
  values only ever becoming text; errors `content.template_not_found`, `content.template_variables_missing`; MCP
  `content.legal_templates`, `content.draft_from_template`. New contract `autoxide_content::api::PublishedEntries`
  (snapshot of a published entry).
- Step 37, website legal profile and the Ministry of Industry and Trade (Vietnam, Decree 52/2013 as amended by Decree
  85/2021 — check the original texts; guide `docs/COMPLIANCE.md`): legal (owner) profiles per tenant and per unit
  (`GET|POST /api/v1/compliance/legal-profiles`, `GET|PATCH /legal-profiles/{id}` with `ETag` / `If-Match`,
  `GET /legal-profiles/{id}/template-variables`; address checked by `address`), websites by domain with kind
  `sales` (notification, blue logo) or `platform` (registration, red logo) (`GET|POST /websites`,
  `GET|PATCH /websites/{id}`), mandatory pages linked to a website (`PUT|DELETE /websites/{id}/pages/{page_kind}`),
  a checklist that is versioned data (rules `moit.checklist.sales` / `moit.checklist.platform`;
  `GET /websites/{id}/checklist`), a draft dossier filled from the data (`GET /websites/{id}/dossier`: declaration
  fields, Markdown drafts of the declaration, service project plan and operating regulations), status and logo recorded
  by hand (`POST /websites/{id}/status`; the logo code is parsed strictly and rebuilt), public
  `GET /public/badge` and `GET /public/owner` for the interface. Permissions `compliance.legal.read`,
  `compliance.legal.manage` (scoped by unit); events `compliance.legal_profile_updated`, `…website_created`,
  `…website_updated`, `…website_status_changed`; MCP `compliance.legal_profiles`, `…websites`,
  `…website_checklist`, `…website_dossier`, `…set_website_status`; contracts `ComplianceFacts` / `Fact` (other modules
  answer checklist items) and `LegalProfiles` (template variables); `autoxide doctor` check
  `compliance.moit_readiness` (warning only). Error codes `compliance.profile_not_found`, `compliance.profile_exists`,
  `compliance.website_not_found`, `compliance.domain_taken`, `compliance.logo_snippet_invalid`. Selling before the
  procedure is done is labelled, not blocked (D-358).
- List exports (step 51): modules declare exportable lists (`StartContext::export_source`, `list_exports`
  module of the kernel); `POST /api/v1/kernel/exports {resource, params, columns}` checks the filters against
  the list's schema and writes a CSV file (protected against CSV injection, `autoxide_kernel::csv`) in a background
  job that reads with the requester's permissions; download it from `GET /api/v1/kernel/jobs/{id}/file` until
  it expires (`AUTOXIDE_JOBS_FILE_RETENTION_DAYS`, 7 days by default). Posts and pages can be exported.
- Filter schemas and saved views (step 51): every list endpoint carries `x-autoxide-list` (its id) and
  `x-autoxide-filters` (name, type, allowed values and operator of each filter) in the API document, generated
  from its own query parameters. `GET/POST /api/v1/me/views` and `PATCH/DELETE /api/v1/me/views/{id}` save list
  views (filters, sort, columns, grouping, density) checked against that schema; views shared with a unit or the
  whole tenant need `kernel.views.share`.
- Background jobs (step 51): modules track the jobs people start (`job::runs::track`, with `progress` and
  `set_result`); `GET /api/v1/kernel/jobs?status=` and `GET /api/v1/kernel/jobs/{id}` show status, progress,
  who asked and the last error (your own jobs, or every job with `kernel.jobs.read`), and `POST
  /api/v1/kernel/jobs/{id}/retry` (`kernel.jobs.manage`) runs a failed job again and records an audit event. The
  CSV export of form submissions is tracked.
- Step 37, account verification for posting (Vietnam, Decree 147/2024 — check the original text): verification levels
  `none` / `phone` / `identity` (identity = verified KYC profile). `GET /api/v1/compliance/me/verification`,
  `POST /me/phone-verifications` (6-digit SMS code through `dyn SmsSender`; Vietnamese mobile numbers only, from the
  new version 2 of the `posting.phone_countries` rule; limited per number, account and IP), `GET
  /me/phone-verifications/{id}`, `POST /me/phone-verifications/{id}/confirm` (single use, expiry, attempt limit,
  `posting.max_accounts_per_phone`), `DELETE /me/phone`; MCP `compliance.account_verification`; events
  `compliance.phone_verified`, `compliance.phone_removed`; new contracts `AccountVerification` and `PostingGate`
  (policies `posting.comments|reviews|livestream`). Error codes `compliance.verification_required`,
  `compliance.otp_invalid`, `compliance.otp_attempts_exceeded`, `compliance.otp_throttled`,
  `compliance.phone_not_allowed`, `compliance.phone_in_use`, `compliance.challenge_not_found`,
  `compliance.sms_timeout`. Settings `AUTOXIDE_COMPLIANCE_OTP_TTL_SECONDS`, `…_OTP_MAX_ATTEMPTS`,
  `…_OTP_PER_PHONE_PER_HOUR`, `…_OTP_PER_USER_PER_DAY`, `…_OTP_PER_IP_PER_HOUR`. `compliance` now registers the
  kernel's `PostingVerifier`, so `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES` checks real phone / identity verification,
  and a tenant's `posting.comments` policy other than `none` makes every comment target need it (kernel:
  `PostingVerifier::required_everywhere`, default `false`). No real SMS provider connector yet (test fake only).
- Step 36 finished — exchange rates, printing, accounting exports, record keeping:
  - Exchange rates with effective dates per currency pair in the ledger: `GET|POST /api/v1/ledger/exchange-rates`,
    `GET /exchange-rates/effective?base=&quote=&on=`, `POST /exchange-rates/fetch` (new kernel trait
    `ExchangeRateSource`, testkit `FakeExchangeRateSource`), MCP `ledger.exchange_rate`, `ledger.record_exchange_rate`.
    Entries in a currency other than the book's are converted with the rate of their day (or the rate the caller
    passes, `NewEntry::exchange_rate`) and keep it on every line; foreign-currency documents keep the rate of their
    issue date (`exchange_rate`, `exchange_rate_id`, `base_total`; `invoices.rate_missing` without one).
    `dyn Ledger` gains `exchange_rate`, `book_currency` and `journal`.
  - Kernel template engine `autoxide_kernel::template` (`minijinja`, sandboxed: no files or other templates, limits
    on size, steps and output, HTML-escaped values, no `safe` filter, variable check on save with every problem at
    once — `template.syntax_error`, `template.unknown_variable`, `template.required_variable_missing`,
    `template.limit_exceeded`); new setting `AUTOXIDE_TEMPLATE_MAX_BYTES`.
  - Print templates per tenant, unit, document kind and language: `GET|POST /api/v1/invoices/templates`,
    `GET|PUT|DELETE /templates/{id}`, `POST /templates/preview`, `GET /template-defaults`; print with
    `GET /documents/{id}/render?format=html|pdf&locale=vi|en` (PDF in a fixed A4 layout with an embedded font for
    Vietnamese), `Invoices::render` for other modules, MCP `invoices.print`. New permission `invoices.templates_manage`.
  - Accounting exports of a period as CSV or XLSX (`generic_journal`, `vn_double_entry`, `generic_documents`):
    `GET /export-profiles`, `GET|POST /exports`, `GET /exports/{id}`, `GET /exports/{id}/download`, job
    `invoices.build_export`, event `invoices.export_ready`, MCP `invoices.create_export`, `invoices.export`, permission
    `invoices.export`. Account numbers follow the Vietnamese chart of accounts (`modules/ledger/data/vn-accounts.json`,
    to be checked by an accountant).
  - Record keeping: `POST|GET /invoices/archives` (permission `invoices.archive`) writes the journal, every document
    and a SHA-256 manifest outside the tenant's folder so it outlives tenant deletion; the daily job
    `invoices.purge_expired` removes consumer buyers' personal data from documents past their retention date and, for
    deleted tenants, deletes expired documents — never one still inside its retention period.
  - Guide `docs/MONEY.md` (and `MONEY.vi.md`): ledger, payments, flow modes, VietQR, documents, exchange rates,
    printing, exports, retention, writing partner connectors.
- Summary figures (step 51): modules declare figures with a mandatory definition (`StartContext::summary_metric`);
  `GET /api/v1/kernel/summary?group=` returns those the caller may see, each with how it is computed in the
  caller's language. Figures: comments waiting, new submissions, entries waiting for a review, media items,
  newsletter subscribers, addresses never mailed.
- Recent activity (step 51): events can carry a readable sentence (`EventInfo::activity`); `GET
  /api/v1/audit/activity` lists them, translated, for callers who may read that kind of record — no `audit.read`
  needed.

- Search across modules (step 51): modules declare finders for their record types
  (`StartContext::finder_source`); `GET /api/v1/kernel/find?q=` returns matches grouped by type (posts and pages,
  media, staff), with or without accents, only from types the caller may read at the chosen unit.

- Work inbox (step 51): modules declare work sources computed from their data (`StartContext::work_source`);
  `GET /api/v1/kernel/work` lists the work from the sources the caller may see, most urgent first (a slow or
  failing source is skipped and named in `incomplete`), `GET /api/v1/kernel/work/counts` counts it by source for
  menu counters. Sources: comments waiting for approval, new form submissions, entries waiting for a review.

- Money flows in the ledger (step 36): units choose how money flows — `self_collect`, `partner_collect` (a licensed
  partner collects for the marketplace) or `split_at_gateway` — with `GET|PUT /api/v1/ledger/flow-settings/{org_id}`
  (inherited down the tree; where cash on delivery goes, never to the marketplace operator). Modules record sales,
  marketplace fees (VAT split), tax withheld for household sellers (rates from the `tax` pack), refunds and partner
  fees through `Ledger::settle`; the seller's share is held until the return window ends (`GET /holds`,
  `POST /holds/{id}/release`, job every 5 minutes) and paid out through the new kernel trait `PayoutGateway`
  (`GET|POST /payouts`, `GET /payouts/{id}`; a refused payout puts the money back). VietQR bank transfers:
  `POST /payments/vietqr` returns the VietQR code and a transfer code, `GET|POST /bank-accounts`, and the new kernel
  trait `BankFeed` reports transfers (`POST /bank-feeds/{provider}` or polling every
  `AUTOXIDE_LEDGER_BANK_FEED_POLL_SECONDS`) that are matched by code and amount; differences go to the exceptions.
  Entries show `sequence` and `sealed_at`; `GET /books/{id}/seal-status`; `Ledger::balances` for many accounts at once.
  New permissions `ledger.settings_manage`, `ledger.payouts_create`, `ledger.holds_release`; events
  `ledger.payout_sent`, `ledger.payout_failed`, `ledger.hold_released`; MCP tools `ledger.flow_settings`,
  `ledger.holds`, `ledger.release_hold`, `ledger.payouts`, `ledger.create_payout`, `ledger.create_vietqr_payment`.
  The testkit has `FakePayoutGateway` and `FakeBankFeed`. Real bank feeds and payout partners are separate modules.
- Navigation for admin interfaces (step 51): `GET /api/v1/kernel/navigation?org_id=` and MCP `kernel.navigation`
  return the groups and screens modules declare (`Module::navigation`), filtered by the modules on for the tenant,
  the caller's permissions at the unit and the plan (`locked` screens need a feature the plan lacks), with labels
  in the caller's language and an `ETag`. Every module declares its screens; platform groups only show to
  platform staff.

- Tenants sign up by themselves (step 35): `POST /api/v1/plans/signups` (no sign-in, anti-spam form token, always
  `202`), a mail with a link valid 24 hours, `POST /signups/confirm` (records acceptance of the platform's terms of use
  and privacy policy in `compliance`), identity checks through `compliance` for plans that `requires_kyc`, then the
  same provisioning staff use — the person is invited as the new tenant's administrator. `GET /signups/{id}?token=`
  follows it; platform staff list sign-ups with `GET /signups` and MCP `plans.list_signups`. Plans get `self_signup`.
  Off by default: new settings `AUTOXIDE_PLANS_SIGNUP`, `AUTOXIDE_PLANS_SIGNUP_URL`,
  `AUTOXIDE_PLANS_SIGNUP_RETURN_URL`. New events `plans.signup_created`, `plans.signup_completed`,
  `plans.signup_rejected`. See `docs/PLANS.md`.
- Entries embed records of other modules (step 51): a body node `embed` with `kind` and `id` (e.g. a product) is
  checked when saved (the type must be registered, the record must exist); public reads return `embeds` keyed
  by `<kind>:<id>` with the title, summary, path and image of each record that is still public.

- New module `notifications` (step 37): notification kinds declared by modules through `Notifier` (security,
  transactional, advertising), sent over email, SMS, Zalo ZNS, web push, mobile push and an in-app inbox
  (`/api/v1/notifications/me/inbox`, unread count, mark read). People choose what they get (`/me/preferences`;
  security notifications cannot be turned off). Advertising needs a consent in the `compliance` ledger, checked again
  right before sending; it waits for the allowed hours and carries the advertising label — both legal rules with
  effective dates (`notifications.send_windows`, `notifications.marketing_labels`) — and advertising emails carry
  `List-Unsubscribe` and a link to `POST /public/unsubscribe`. Tenant texts per kind, channel and language
  (`/templates`), a send log (`/messages`), push subscriptions and device tokens (stored encrypted), and notices from
  platform staff to the admins of every tenant (`POST /platform-notices`); the `plans` module uses it to tell tenant
  admins when a subscription is past due or suspended or close to a limit. Kernel traits `ZnsSender`,
  `WebPushSender`, `MobilePushSender` (providers are separate modules; the testkit has fakes) and `auth` trait
  `TenantAdmins`. New settings `AUTOXIDE_NOTIFICATIONS_RETENTION_DAYS`, `…_INBOX_RETENTION_DAYS`,
  `…_UNSUBSCRIBE_URL`, `…_PER_RECIPIENT_PER_HOUR`. See `docs/NOTIFICATIONS.md`.
- `GET /api/v1/kernel/whats-new` (no sign-in): the changelog of the running server by version. `GET
  /api/v1/kernel/ops/alerts` (platform staff): the `autoxide doctor` checks that are not fine, with what to do.
- Tenant setting `kernel.business_model` (`independent`, `multi_store`, `chain`, `marketplace`, `saas`). Tenant
  setting `kernel.public_url` now takes effect: links in mails sent for the tenant (sign-in links, invitations,
  password resets, email checks, newsletter and comment confirmations) open on the tenant's site.

- `GET /api/v1/audit?actor_kind=user|system|public`; `GET /api/v1/newsletter/subscribers?sort=email|status` (emails
  sort byte by byte, the same on SQLite and PostgreSQL).

- New module `compliance` (step 37): identity verification (KYC) through a provider (`IdentityVerifier`, callbacks
  checked by signature, processed in the background) or by manual review (signed-in reviewers only); duplicate
  documents wait for review; only the result, the provider reference, the verified name and the last four digits are
  kept. Legal rules are dated data (bundled `vn` pack, imported by platform staff with `POST /api/v1/compliance/rules`);
  selling, livestreaming and affiliate marketing require a verified identity from 1 July 2026. Versioned legal documents
  with translations, a SHA-256 of each text and proof of acceptance (`/api/v1/compliance/documents`,
  `/me/pending-acceptances`, `/me/acceptances`, public `GET /public/documents/{kind}`); a consent ledger per purpose
  (`/me/consents`). Other modules use it through `KycGate`, `AcceptanceGate`, `ConsentLedger` and `ComplianceRules`.
  New setting `AUTOXIDE_COMPLIANCE_KYC_CALLBACK_RETENTION_DAYS`. The legal bases are recorded but must be checked
  against the original texts by a legal professional.

- Admin interface requests (W-059 of the web): `q` on `GET /api/v1/content/redirects` (old path or target, any case,
  with or without accents); `ids` on `GET /api/v1/forms/{id}/submissions/export` to export picked submissions;
  `POST /api/v1/forms/{id}/duplicate`; `POST /api/v1/auth/users/{id}/email/verification` to send a user's
  verification mail again.

- Subscription billing (step 35): each billing period becomes an invoice the operator issues to the tenant
  (numbered with no gaps, taxes computed — `AUTOXIDE_PLANS_TAX_CATEGORY`, booked in the operator's ledger); tenants
  pay online with `POST /api/v1/plans/billing-periods/{id}/pay` (the operator's payment partner, checkout page
  returned), and the period is marked paid when the partner reports the payment. Billing periods show their
  `invoice_id`. Voiding a period cancels its invoice. `plans` is now registered after `invoices`.

- Admin interface requests (17/11): `created_by_name` on content revisions (`GET /api/v1/content/entries/{id}/revisions`
  and one revision); `GET /api/v1/menus/{id}?include=targets` says how the object of each item stands (`public`,
  `not_public`, `missing`, `unknown_kind`, with the title and path of public ones); `POST /api/v1/menus/import-content`
  imports the old menus of module `content` over REST, with `dry_run`.

- New module `invoices` (step 36): accounting documents. Series (symbols) per unit, kind and year
  (`/api/v1/invoices/series`); draft documents with taxes computed by the tax engine (`/api/v1/invoices/documents`);
  issuing gives the next number of the series with no gaps, even when issued at the same time, books the sale in the
  ledger and sends the e-invoice through a provider (kernel trait `autoxide_kernel::einvoice::EInvoiceProvider`,
  `FakeEInvoiceProvider` in the testkit) — also cash register invoices, and invoices issued on behalf of a seller under
  a mandate (`/api/v1/invoices/mandates`). A refused e-invoice is sent again with the same number; issued documents
  are corrected by adjustment or replacement, or cancelled, and are never deleted (kept at least 10 years). MCP
  `invoices.documents`, `invoices.document`, `invoices.create_draft`, `invoices.issue`. Other modules use
  `autoxide_invoices::api::Invoices`.

- Payments through licensed partners (step 36): kernel trait `autoxide_kernel::payment::PaymentGateway` (connector
  modules contribute it; tests use `FakePaymentGateway` from the testkit). The `ledger` module creates payments
  (`POST /api/v1/ledger/payments`, the partner's checkout page comes back; sending the same `reference` again after a
  partner timeout retries safely), receives signed partner webhooks (`POST /api/v1/ledger/webhooks/{provider}`,
  checked on the raw body, stored once, processed in the background), books collected and refunded money,
  refunds (`POST /api/v1/ledger/payments/{id}/refunds`, never more than collected), expires unpaid payments
  (`AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES`) and reconciles each day with the partner statement
  (`POST /api/v1/ledger/reconciliations`); differences wait in `GET /api/v1/ledger/exceptions` for a person. Events
  `ledger.payment_succeeded` and friends let modules react. MCP `ledger.payment`, `ledger.exceptions`,
  `ledger.resolve_exception`. The server never holds buyers' money.

- New module `ledger` (step 36): a double-entry ledger. Each organization unit keeps a book; accounts by purpose
  (`cash.bank`, `revenue.sales`, `payable.seller_held`… or `<module>.<name>`), per counterparty and currency, with an
  optional floor; entries are append-only (debits equal credits, one currency) and corrected by reversing them
  (`POST /api/v1/ledger/entries/{id}/reverse`); balances stay exact under concurrent writes on SQLite and Postgres.
  Monthly periods close for good (`POST /api/v1/ledger/periods/{id}/close`) with closing balances kept, balances at a
  day (`GET /api/v1/ledger/accounts/{id}/balance?as_of=`) and a trial balance (`GET /api/v1/ledger/trial-balance`).
  Entries are sealed in a hash chain; `autoxide doctor` reports a changed or removed entry (`ledger.chain`) and
  balances that differ from their lines (`ledger.balances`). Ledger tables are kept when a tenant is deleted for good
  (accounting records). MCP `ledger.balance`, `ledger.entries`, `ledger.trial_balance`, `ledger.post_entry`. Other
  modules post through `autoxide_ledger::api::Ledger`.

- New module `tax` (step 36): taxes from data. Tax categories and rates with effective dates, withholding rates
  for platforms and rules come in a tax data pack; the Vietnam pack ships with the server (VAT 0/5/10%, the 8% rate
  until 2026-12-31, a special consumption tax sample, marketplace withholding for household businesses — every rate
  with its legal basis, to check before production use) and is imported by itself. `POST /api/v1/tax/quote` computes
  the taxes of a document (exclusive or inclusive prices; rounded once per rate, lines always add up), `POST
  /api/v1/tax/withholding` the taxes a platform withholds; tax profiles per organization unit, inherited down the
  tree (`/api/v1/tax/profiles/{org_id}`); `GET /api/v1/tax/rates`, `/withholding-rates`, `/rules`, `/packs`; MCP
  `tax.quote`, `tax.rates`; `autoxide task tax.import_pack`; `autoxide doctor` warns about rates ending with no rate
  after them (`AUTOXIDE_TAX_EXPIRY_WARNING_DAYS`). Other modules use `autoxide_tax::api::TaxEngine`.

- Create a tenant on a plan in one call: `POST /api/v1/plans/provisionings` (platform staff, new permission
  `plans.provision`; MCP `plans.provision_tenant`, `plans.get_provisioning`) creates the tenant, its subscription, the
  sample data of the modules the plan turns on, and invites its administrator; it runs in the background, failed
  steps can be retried (`POST …/{id}/retry`), and `GET /api/v1/plans/provisionings` lists them. From the server:
  `autoxide tenant create … --plan <id> --admin-email …`. Modules supply sample data with `Module::sample_data`.

- Delete a tenant and its data for good: `POST /api/v1/kernel/tenants/{id}/erase` (platform staff with the new
  permission `kernel.tenants.delete`; the tenant must be suspended, and its slug sent again as `confirm`). Rows and
  files of the tenant are deleted, its domains released; the tenant stays listed with status `deleted` and requests
  to it answer `404 tenant_not_found`. Tables modules must keep (accounting records) are declared with
  `Module::retained_tables`. A subscription that reaches the end of its retention period now deletes the tenant this
  way (it was only suspended).

- Billing profile of a tenant (legal name, tax code, email for invoices, phone, address):
  `GET/PUT /api/v1/plans/billing-profile`, and `GET /api/v1/plans/tenants/{tenant_id}/billing-profile` for platform
  staff. Billing reminders go to its email when the subscription has no `billing_email`.
- For platform staff: `GET /api/v1/plans/tenants/{tenant_id}/billing-periods`, `GET
  /api/v1/plans/tenants/{tenant_id}/entitlements`, `GET /api/v1/plans/overrides` (every tenant), `subscriber_count` on
  plans, `tenant_id` filter on `GET /api/v1/plans/subscriptions`. Subscriptions include `plan_code` and `plan_name`.
- `GET /api/v1/kernel/tenants?expand=subscription,usage` now fills `subscription` and `usage` when the `plans` module
  is installed (they were always `null`).
- `GET /api/v1/auth/support-grants` without `tenant_id` lists the newest grants of every tenant;
  `GET /api/v1/auth/support-grants/allowed?tenant_id=` tells whether a tenant lets platform staff in;
  `GET /api/v1/kernel/client-config` has `auth.support_access_max_hours`.

- `POST /api/v1/kernel/placements/migrations` (platform staff, MCP `kernel.migrate_placements`): migrate the tenant
  databases from the API — `dry_run` lists the pending migrations, otherwise a run goes on in the background
  (`GET …/migrations/{id}`, `GET …/migrations`). One run at a time.
- `GET /api/v1/auth/roles/counts` (MCP `auth.count_role_members`): how many people hold each role.
- `PATCH /api/v1/kernel/tenants/{id}` takes a `reason`, kept with the change in the audit log; suspending a tenant
  from `plans` now records its reason there too.
- `GET /api/v1/auth/me/memberships` (and `/users/{id}/memberships`) include `org_name` and `org_unit_type`, so a user
  who cannot read the organization tree still sees where they work.
- `GET /api/v1/auth/delegations/tenants`: a parent tenant lists the tenants below it, to choose where a delegation
  goes.
- `GET /api/v1/kernel/client-config` has an `auth` section: `login_link`, `passkeys`, `self_registration`, so a
  sign-in page shows only the ways the server allows.
- Support access grants include `staff_name`, so a tenant sees who may enter its data.
- `GET /api/v1/auth/memberships/counts` (MCP `auth.count_memberships`): how many people work in every organization
  unit — `direct` and `total` (including people on the units above) — without reading every membership.
- `GET /api/v1/organizations/settings` gives each setting's `schema` (JSON Schema of its value: `type`, `enum`,
  `pattern`…), so interfaces show the right input. Modules declare it with `OrgSetting::with_schema`.
- `GET /api/v1/organizations/public/site`: the public profile of the website being visited — the unit its domain is
  bound to (or `?site=`), else the whole tenant — so a store's own domain shows the store, not the head office.
- Plans can cap organization units per unit type: `"organizations.units.store": 2` next to the overall
  `organizations.units`. Modules declare such limits with `Limit::per` and reserve parts with
  `reserve_part` / `confirm_part` / `release_part`; `GET /api/v1/kernel/modules` shows `parts`, and
  `GET /api/v1/plans/entitlements` lists the per-part use.
- For module authors (step 36): `autoxide_kernel::numbering` hands out gapless sequential numbers (invoice and receipt
  numbers) inside the caller's write transaction — concurrent requests get 1, 2, 3… with no duplicates, and a rolled
  back transaction does not use up a number; `rows::lock_one_shared` takes a shared row lock; `StartContext::contribute`
  / `contributions` let several modules supply the same trait (e.g. one payment gateway connector each).
- Export all the data of a tenant as one JSON Lines file, in the background: `POST /api/v1/kernel/tenants/{id}/exports`
  (platform staff, or the tenant's own administrators with the new permission `kernel.data.export`), status, download
  with a bearer token or a 24-hour link; MCP tools `kernel.export_tenant`, `kernel.get_tenant_export`. Secrets and
  password hashes are left out; started automatically when a subscription is canceled; kept
  `AUTOXIDE_TENANCY_EXPORT_RETENTION_DAYS` (30).
- New module `plans` (step 35): sell the software by plan without code changes. A plan is data — modules, features,
  limits and prices — with add-ons and per-tenant overrides; tenants without a subscription get the default plan, and
  with no default plan nothing changes. `GET /api/v1/plans/entitlements` tells a user interface what the tenant may
  use; platform staff manage plans and subscriptions under `/api/v1/plans` (`plans.plans.*`,
  `plans.subscriptions.*`) and over MCP. Modules declare features and limits (`Module::features`, `Module::limits`),
  and reserve, confirm and release limits through the kernel's `EntitlementsHandle`; reservations are atomic on
  both databases. The kernel enforces the plan: endpoints of a module the tenant's plan does not include answer
  `403 module_not_enabled`, its MCP tools are hidden, its jobs wait (`AUTOXIDE_JOBS_BLOCKED_DELAY_SECONDS`, not counted
  as attempts) and its event handlers are skipped; a suspended or canceled subscription makes the tenant read-only
  (`403 tenant_read_only` on writes). Limits wired so far: `organizations.units` (units besides the root, archived ones
  not counted), `auth.members` (people holding a role), `files.storage_bytes`, `kernel.api_requests` (requests per
  month, recorded every minute; `429 quota_exhausted` only when the plan sets a maximum); features `auth.api_keys` and
  `content.ai_suggestions`. Held reservations expire and usage is reconciled with the real counts daily.
  Subscriptions have a lifecycle run by jobs — trial, active, past due (grace), suspended (read-only), canceled
  (read-only, data kept), deleted — with billing periods paid by hand for now (`mark-paid`, `void`), reminder mails,
  self-service change (upgrades at once with a prorated period, downgrades at period end), cancel and resume, and a
  way for platform staff to suspend a tenant completely. The tenant list filters by `subscription_status` and
  `near_limit`. Operator guide: `docs/PLANS.md`. The module catalog lists each module's features, limits, `always_enabled` and `requires`.
- Decimal numbers for calculating with money (tax rates, exchange rates, quantities) in
  `autoxide_kernel::decimal`: `Money::apply_rate`, `Money::convert`, `Money::to_decimal` / `from_decimal`, each with
  an explicit `Rounding`; decimals are strings in JSON (`DecimalString`, error `decimal.invalid`) and text in the
  database. Amounts are still stored as integers of minor units.
- Security headers on every response, including errors, health checks and `/mcp`:
  `X-Content-Type-Options: nosniff`, `Referrer-Policy` (`AUTOXIDE_HTTP_REFERRER_POLICY`, default `no-referrer`),
  `X-Frame-Options: DENY` and a strict `Content-Security-Policy` (the API reference page gets its own);
  `Strict-Transport-Security` when `AUTOXIDE_HTTP_HSTS_SECONDS` is set. A header a handler sets itself is kept;
  `AUTOXIDE_HTTP_SECURITY_HEADERS=false` turns them off. `autoxide doctor` reminds you of HSTS when the public URL
  is `https://`. Downloads from `files` and `media` now carry the stricter common policy.
- New module `address`: the official list of administrative units shared by every tenant
  (Vietnam after the 2025 merger: provinces and communes, no districts), imported from the
  official source with `autoxide task address.import path=<dir>` (a new list closes and opens
  units by date; importing again changes nothing); public `GET /api/v1/address/provinces`,
  `/provinces/{code}/communes`, `/units`, `/units/{code}` (search without accents, `ETag`, one-day
  cache), `POST /api/v1/address/validate`; MCP tools; `autoxide doctor` shows the list in use. See
  `docs/ADDRESS.md`. Ships the Vietnamese list in force from 2025-07-01 (34 provinces, 3,321 communes):
  import it with `autoxide task address.import`.
- `organizations`: public profile of a unit — `GET`/`PATCH /api/v1/organizations/units/{id}/public-profile`
  (`organizations.profile.manage`): shown or not, display name, checked address, opening hours,
  special days, pickup, cover image; public `GET /api/v1/organizations/public/stores` (`ETag`);
  object type `organizations.store` for website menus; event `organizations.public_profile_updated`.

- `auth`: platform staff enter a tenant's data only with a time-limited support access grant
  (D-190): `POST`/`GET /api/v1/auth/support-grants`, `DELETE /api/v1/auth/support-grants/{id}`
  (`auth.support.grant`, platform tenant), `GET /api/v1/auth/support-access`,
  `DELETE /api/v1/auth/support-access/{id}` for the tenant (`auth.support_access.read`,
  `auth.support_access.revoke`), setting `auth.support_access`, `AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS`;
  events `auth.support_access_granted`, `started`, `revoked` in the tenant's audit log.

- `auth`: invite people by email with roles at a unit — `GET`/`POST /api/v1/auth/invitations`,
  `DELETE /api/v1/auth/invitations/{id}`, `POST /api/v1/auth/invitations/{id}/resend`,
  `POST /api/v1/auth/invitations/accept` (permission `auth.invitations.manage`; settings
  `AUTOXIDE_AUTH_INVITATION_URL`, `AUTOXIDE_AUTH_INVITATION_DAYS`), MCP tools
  `auth.create_invitation`, `auth.list_invitations`, `auth.revoke_invitation`, and the command
  `autoxide user invite --email … --role … [--org …] [--tenant …]`. Events
  `auth.invitation_created`, `auth.invitation_accepted`, `auth.invitation_revoked`.

- `auth`: sign in with a link sent by email (off by default: `AUTOXIDE_AUTH_LOGIN_LINK`,
  `AUTOXIDE_AUTH_LOGIN_LINK_URL`, `AUTOXIDE_AUTH_LOGIN_LINK_MINUTES`) — `POST /api/v1/auth/login/link`
  (same answer whether or not the email exists; optional `bind_device`) and
  `POST /api/v1/auth/login/link/confirm` (two-step sign-in still applies). Event
  `auth.login_link_used`; sessions show `sign_in_method: email_link`.

- `auth`: customer accounts (`realm: customer`) separate from staff in each tenant — register and
  sign in with `"realm": "customer"` (setting `auth.customer_registration` per tenant), no
  permission; `GET /api/v1/auth/customers`, `GET`/`PATCH /api/v1/auth/customers/{id}`
  (`auth.customers.read`, `auth.customers.manage`). Password reset takes `realm`.
- `auth`: delegation down a chain of tenants — `GET`/`POST /api/v1/auth/delegations`,
  `DELETE /api/v1/auth/delegations/{id}` (`auth.delegations.read`, `auth.delegations.manage`): holders
  of a parent's role work in a child with the header `Autoxide-Tenant`; `GET /api/v1/auth/me/tenants`
  lists where you can work. Events and the audit log record how the person came in
  (`actor_access`: `direct`, `delegation`, `support`). For module authors: every module's events
  table gets the migration `outbox_access_migration`; `Authenticator::enter` decides who may enter
  another tenant.

- `auth`: `POST /api/v1/auth/me/reauth` gives a short-lived, single-use confirmation token (by
  password, authenticator code, passkey or a mail) that sensitive changes accept as `reauth_token`
  instead of `current_password` — creating an API key, changing the email, turning on two-step
  sign-in, adding a passkey, deleting the account (new setting `AUTOXIDE_AUTH_REAUTH_MINUTES`).
  `PATCH /api/v1/auth/me` changes your name, preferred language and time zone; `GET /api/v1/auth/me`
  gives `locale`, `time_zone` and `mfa_required`. Roles take `requires_mfa` (also the built-in
  `admin` role): their permissions apply only once the account has two-step sign-in. Mails sent
  later use the preferred language.

- Websites of organization units have their own addresses and languages: two units can both
  publish `/about` (the website shows the one of the nearest unit; slugs and addresses are unique
  per unit), and the new setting `organizations.languages` (list, first is the default) sets the
  languages a unit's website offers — used by the public endpoints of `content` (posts, resolve,
  sitemap, feed, search) and `menus`. Public search (`GET /api/v1/content/public/search`) takes
  `site` and shows only what that website shows.

- `organizations`: settings that units inherit down the tree. `GET /api/v1/organizations/settings`
  lists them; `GET /api/v1/organizations/units/{id}/settings` gives the value in force at a unit and
  where it comes from (the unit, a unit above, or the default); `PUT` and `DELETE
  /api/v1/organizations/units/{id}/settings/{key}` set and clear one (new permission
  `organizations.settings.write`, new event `organizations.setting_changed`; MCP tools
  `organizations.get_settings`, `organizations.set_setting`). Three settings to start:
  `organizations.timezone` (IANA, default `UTC`), `organizations.currency` (ISO 4217) and
  `organizations.language` (`en`, `vi`; default `en`). For module authors: `Module::org_settings`
  declares more, `OrgDirectory::setting` reads the value in force.

- `auth`: `GET /api/v1/auth/users` takes `status` (`active`, `disabled`) and `include=roles` (each user's roles in
  the same answer); every user carries `has_password`, `mfa_enabled` and `last_sign_in_at`. `GET /api/v1/auth/me`
  gives `has_password` and `pending_email` (an email change waiting for its link, with expiry);
  `DELETE /api/v1/auth/me/email` cancels that change. New event `auth.email_change_cancelled`. Users are an object
  type `auth.user` of the object directory (their display name), for other modules.
- `auth`: each session gives `sign_in_method` and an approximate `country` (ISO code from a trusted
  proxy header, new setting `AUTOXIDE_AUTH_COUNTRY_HEADER`; the IP is never kept); each passkey
  gives `aaguid` and `authenticator` (e.g. `Windows Hello`); `GET` and `DELETE
  /api/v1/auth/all-api-keys…` let holders of the new permission `auth.api_keys.manage` see and
  revoke every user's keys; creating an API key accepts `totp_code` or a passkey (`passkey_ceremony_id`,
  `passkey_credential`) instead of `current_password`.
- `GET /api/v1/kernel/client-config` (no sign-in): the settings an interface needs, by module —
  `kernel` (public address, version), `content` (languages, post path prefix, embed hosts),
  `media` (upload limit and types, alt languages), `menus` (languages), `mail` (sender, whether
  mail is really sent). For module authors: `Module::client_config` with `ClientSection`.
- `comments`: comments in the moderation list and when read carry `target_title` and `target_url`
  (from the module that owns the target, when public); `GET /api/v1/comments/targets` lists the
  targets with comments, their counts, title and path, filtered by kind, state or pending.
- `newsletter`: `GET /api/v1/newsletter/settings` (the settings in use and what is still missing
  to send); `GET /api/v1/newsletter/campaigns/{id}/preview` (the mail as sent, with footer);
  campaign `progress` gives `bounced` and `unsubscribed`; subscribers filter by `blocked`.
- `forms`: forms carry `submissions` (counts by status) in the list and when read; each answer to
  a choice field carries `choices`, the chosen options with their labels in the form version it was
  sent with; `value` of an answer is typed in the contract (`string | number | string[] | boolean`).
- `media`: a `decorative` flag on items (an image whose empty alt text is on purpose);
  `GET /api/v1/media` filters by `alt_missing` and takes `include=uploader` (`uploaded_by_name`);
  `GET /api/v1/media/summary` gives the totals of the library; folders carry `item_count`.
- `content`: `GET /api/v1/content/entries` filters by `review_requested`, `scheduled`, `created_by` (`me` for the
  caller), `ai_generated` and `missing_locale`; each entry carries `published_at`, `path` (of the public version)
  and `version` (the `ETag` value). Categories, tags and authors carry `entry_count`; categories and authors take
  `q` (accents ignored). `GET /api/v1/content/schedules` lists the schedules of every entry; a schedule takes
  `ai_reviewed`. SEO check messages follow `Accept-Language`; a preview gives the addresses of its public images.
- `audit`: `include=actor` adds `actor_name`, the display name of who made each change.
- `webhooks`: each endpoint carries `last_delivery` (status, HTTP status, time) in the list and when read;
  `status` (`active`, `disabled`) filters the list.
- `GET /api/v1/me/permissions`: the caller's own permissions with description and risk, for anyone signed in
  (e.g. to pick the permissions of a new API key without `auth.roles.read`).
- `GET /api/v1/auth/permissions` gives the `risk` of each permission. Permission descriptions (there, in
  `GET /api/v1/kernel/modules`) and event descriptions (`GET /api/v1/webhooks/events`) follow `Accept-Language`.
- Problem responses may carry `reason`: a refused link token (`auth.invalid_verification_token`,
  `auth.invalid_reset_token`, `auth.invalid_email_change_report_token`) says `expired`, `used` or `unknown`.
- `menus`: `POST /api/v1/menus` answers with an `ETag`; an item sent to `PUT /api/v1/menus/{id}` with the `id` it had
  keeps that id; the menu list gives `item_count` and `updated_by`.
- The text of each field problem (`errors[].detail` of a `422` Problem, and `errors` of an MCP tool error) follows
  `Accept-Language` like `detail` already did. For module authors: `FieldError::new` takes an English template with
  `{name}` placeholders filled by `.with_param(name, value)`; its translation is keyed by the template
  (`"field:<template>"` in `locales/vi.json`), and `cargo xtask check` refuses a message built with `format!` or one
  without a translation.
- Foundations for serving many customers (tenants) from one installation: the control-plane tables
  `kernel_tenants`, `kernel_placements` and `kernel_tenant_domains` (the existing data belongs to the default tenant),
  and the settings `AUTOXIDE_TENANCY_*`. The default mode `single` changes nothing; `multi` is refused until every
  module keeps its data per tenant.
- Tenant management for platform staff (people of the default tenant): `GET /api/v1/kernel/tenant` (the caller's
  tenant, any signed-in user), `GET`, `POST /api/v1/kernel/tenants`, `GET`, `PATCH /api/v1/kernel/tenants/{id}`
  (rename, suspend, resume; `If-Match`), `GET /api/v1/kernel/placements` (connection URLs are never shown); MCP tools
  `kernel.current_tenant`, `kernel.list_tenants`, `kernel.get_tenant`; CLI `autoxide tenant create|list|show|suspend|
  resume|provision`. New permissions `kernel.tenants.read`, `kernel.tenants.manage`, `kernel.placements.read`, new
  events `kernel.tenant_created`, `kernel.tenant_updated`. Creating a tenant is refused in the `single` mode.
- Placements — the databases that keep tenants' data: `autoxide placement add <name> --url …` (the URL is checked
  by connecting, then stored encrypted; `--url -` reads it from standard input), `autoxide placement list`,
  `autoxide placement check`. `autoxide migrate up --all` / `status --all` run the main database, then every
  placement by name, and can be run again after a failure. While a placement has migrations to apply, only the
  tenants on it answer `503` (`tenant_maintenance`) and its jobs wait; `serve` checks the placements every minute in
  the multi-tenant mode. New error codes `tenant_placement_taken`, `tenant_placement_behind` (creating a tenant on
  a placement that is not up to date).
- For module authors (testing): `TestApp::builder(…).tenant("a")` (a tenant on the shared database),
  `.tenant_dedicated("b")` (a tenant on a database of its own), `.user_in("a", token, id, permissions)`, then
  `app.tenant("a")` / `app.default_tenant()`; `autoxide_testkit::leak_check(&app, token_of_b, marker, ids)` calls every
  `GET` endpoint and every read-only MCP tool as a user of another tenant and lists what shows data of the first.
- `notes` (the example module) keeps its notes per tenant: the same title may exist in two tenants, a note of one
  tenant is "not found" in another; existing notes belong to the default tenant.
- For module authors: `Module::seed_tenant` gives the data a new tenant starts with (roles, counters…); it must be
  safe to run again, because `autoxide tenant provision` repeats a creation that stopped half-way.

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
- For module authors: `autoxide_kernel::storage::TenantStorage` keeps one tenant's files under its own prefix
  (none for the default tenant, so existing files stay where they are; `tenants/<id>/` for others) and refuses
  keys that climb out of it; `SecretKey::derive_for(tenant, purpose)` gives a signing key per tenant (the default
  tenant keeps the key `derive` gives).
- Several tenants in one installation: `AUTOXIDE_TENANCY_MODE=multi` (needs `AUTOXIDE_SECRET_KEY`). The default
  `single` mode needs nothing new and runs no extra query per request. A request reaches a tenant by its credential,
  by a verified domain, or (before sign-in, and for platform staff with `kernel.tenants.operate`) by the
  `Autoxide-Tenant` header. `cargo xtask new-module` generates a module that keeps its data per tenant.
- Custom domains of tenants: `GET`, `POST /api/v1/kernel/domains`, `PATCH`, `DELETE /api/v1/kernel/domains/{id}`,
  `POST /api/v1/kernel/domains/{id}/verify` (permission `kernel.domains.manage`; MCP tools `kernel.list_domains`,
  `kernel.add_domain`, `kernel.update_domain`, `kernel.verify_domain`). A domain is verified by a DNS TXT record,
  checked again every day, and may belong to one node of the organization tree. `GET
  /api/v1/kernel/domains/allowed?domain=…` answers a TLS proxy (Caddy `on_demand_tls`) whether to request a
  certificate. New events `kernel.domain_added`, `kernel.domain_verified`, `kernel.domain_updated`,
  `kernel.domain_failed`, `kernel.domain_revoked`. See `docs/DEPLOY.md`.
- Moving a tenant between databases (`shared` ↔ `dedicated`, SQLite and PostgreSQL in either direction):
  `autoxide tenant move <tenant> --to <placement>` (`--dry-run` first, `--keep-source`, `--abort`),
  `autoxide tenant purge-copy`, and `POST /api/v1/kernel/tenants/{id}/move` (a dry run unless confirmed) with
  `GET /api/v1/kernel/tenants/{id}/moves/{move_id}` (permission `kernel.tenants.move`; MCP tools
  `kernel.move_tenant`, `kernel.get_tenant_move`). The tenant answers `503 tenant_maintenance` while it moves; a
  move that stopped half-way resumes. New event `kernel.tenant_moved`.
- A placement can keep the files of its tenants in its own storage: `autoxide placement storage <name> --dir …`
  or `--s3-bucket …` (stored encrypted), `--clear` to go back to the shared storage.
- `autoxide backup` in the `multi` mode says which placements it does not copy.
- `scripts/loadtest.sh` measures the multi-tenant mode too: `LOADTEST_TENANCY=multi` starts the server with 100
  tenants on the main database and 10 tenants with SQLite databases of their own, then measures reads, lists and
  writes of a user of each kind. Results before and after tenancy are in `docs/BENCHMARKS.md`.
- Module guide (`docs/MODULE_GUIDE.md`): a section on tenants — how to classify tables, filter every query with
  `.scoped`, seed a new tenant, read per-tenant settings and write the leak test.
- Organization tree of each tenant (new module `organizations`): unit types the tenant names itself
  (`GET`, `POST /api/v1/organizations/unit-types`, `PUT`, `DELETE /api/v1/organizations/unit-types/{code}`) and units
  under a root that has the id of the tenant (`GET`, `POST /api/v1/organizations/units`, `GET`, `PATCH
  /api/v1/organizations/units/{id}`, `GET …/{id}/ancestors`, `POST …/{id}/move`, `POST …/{id}/status`). A unit is
  `pending`, `active`, `suspended` or `archived`; while a unit is not active, nothing can be written to it or to any
  unit below it. The tree is at most `AUTOXIDE_ORGANIZATIONS_MAX_DEPTH` levels deep (default 8). New permissions
  `organizations.units.read`, `organizations.units.write`, `organizations.units.manage` (scoped) and
  `organizations.types.manage`; MCP tools `organizations.list_units`, `get_unit`, `create_unit`, `update_unit`,
  `move_unit`, `change_unit_status`; events `organizations.unit_created`, `unit_updated`, `unit_moved`,
  `unit_status_changed`, `unit_type_changed`. Other modules read units through `autoxide_organizations::api::OrgDirectory`.
- For module authors: permissions with a depth. `Permission::scoped()` marks a permission on data that belongs to a
  unit; a role grants it with a scope `own`, `unit`, `subtree` or `tenant` (`autoxide_kernel::access::Scope`).
  `Principal::scope_condition(&permission, org_column, owner_column)` limits a list, `require_at` checks one record
  and `require_write_at` also refuses a unit that is not active (`org_frozen`). `Principal::with_grants`,
  `with_fence` (an API key bound to a unit), `customer` (an account without permissions), `realm`, `access`. The
  structure of the tree is in the kernel: `autoxide_kernel::tenancy::{OrgTree, nearest, root_of}`. New error codes
  `org_out_of_scope`, `org_frozen`, `org_not_found`, `org_cycle`, `org_depth_exceeded`, `org_root_immutable`. Testkit:
  `TestApp::builder(…).member(token, user, org, &[(permission, scope)])`.
- `auth`: people work in organization units. `GET`, `PUT /api/v1/auth/users/{id}/memberships` (the units a user works
  in and the roles there), `GET /api/v1/auth/memberships` (filter by `org_id`, `role_id`, `user_id`; `inherited=true`
  also counts memberships on the units above), `GET /api/v1/auth/me/memberships`; MCP tools `auth.list_memberships`,
  `auth.set_memberships`; event `auth.memberships_changed`. Roles take `grants: [{permission, scope}]` next to
  `permissions` (scope `own`, `unit`, `subtree` or `tenant`) and return both; `GET /api/v1/auth/permissions` gives
  `scoped`. An API key can be bound to a unit (`org_id`): it then acts only on that unit and the units below it.
  Existing roles, role assignments and keys keep their meaning (the whole tenant); `PUT /users/{id}/roles` sets the
  roles at the root unit.
- Content, media, forms and menus per organization unit and website. Entries, media items and menus belong to a unit
  (`org_id` when creating; existing data belongs to the root), forms may (`null`: the whole tenant). Their permissions
  follow the scope of the role; data outside the scope answers 404. Admin lists filter by `owner_org` (and entries by
  `site`). Public endpoints (`/api/v1/content/public/resolve`, `/posts`, `/sitemap`, `/feed`,
  `/api/v1/menus/public/{key}`) serve the website of the request: a domain bound to a unit, or `?site=<unit id>`;
  a website shows the content of its unit and of the units above it, and the menu of the nearest website that has one.
  New error codes `site_not_found`, `invalid_site`. For module authors: `autoxide_kernel::tenancy::{Site, SiteQuery}`,
  `Principal::home_org`, `OrgTree::ensure_writable_unlocked`.
- For module authors (testkit): `branch_leak_check(&app, token, marker, ids)` — the leak test between two sibling
  units of one tenant: a person of one branch calls every `GET` and read-only MCP tool and must see nothing of the
  other. Module guide: a section "Data of organization units" (`org_id`, `.scoped()`, `scope_condition`,
  `require_write_at`, inherited settings, the branch leak test).
- For module authors (testkit): `MemoryStorage`, an in-memory `dyn Storage` with `fail_writes` to test storage
  errors.

### Changed

- Counting failed attempts (sign-in lockout and other throttles) takes one SQL statement instead of up to four:
  60–75% more attempts per second, except many attempts on one email at once on PostgreSQL, which still queue on
  that email's counter.
- Faster signed-in requests: the user and their permissions are read in one SQL query instead of two on every
  request (access token and API key); `GET /api/v1/auth/me` and the MCP tool `auth.whoami` need three queries in
  all instead of seven, and `GET /api/v1/auth/users` one query per page for the account facts instead of two.
  Responses are unchanged.
- `notes` (the example module) keeps its notes per organization unit: each note has `org_id` (existing notes belong to
  the root unit), `notes.read` and `notes.write` follow the scope of the role (`own`: only your own notes — what they
  meant before; `unit`, `subtree`; `tenant`: every note), and creating a note takes `org_id` (`notes.org_required`
  when you work in several units and give none). A role that grants `notes.read` for the whole tenant now sees every
  note of the tenant: give it with scope `own` to keep personal notes.
- The `detail` of some errors no longer names an endpoint: it says what the user can do (`auth.wrong_password`,
  `auth.password_already_set`, the token errors of `auth`, `media.item_in_trash`…). Which endpoint to call is in the
  description of the response in `docs/openapi.json`. Codes are unchanged.
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
- Form tokens (`GET /api/v1/antispam/token`) and `Idempotency-Key` scopes belong to one tenant: a token or a key
  of one tenant means nothing in another. `SpamGuard::issue` takes the tenant. Nothing changes for the default
  tenant: tokens and keys already handed out keep working.
- Interface flags (`/api/v1/me/flags`) are kept per tenant: the same account in two tenants has two sets.
- Internal, no change in behaviour: the code of `newsletter`, `comments`, `media` and `forms` is grouped by
  feature like `content` and `auth`, and the kernel's files sit in private groups (`contracts`, `data`,
  `security`, `ops`, `values`). Every public path (`autoxide_kernel::ai::TextModel`, …) is unchanged, and a
  test keeps it so.
- For contributors: the tests run about twice as fast. Dependencies are built with `opt-level = 1` (and the password
  hash `argon2` with `opt-level = 3`) in dev and test builds — your own code stays unoptimised, so rebuilds take
  as long as before. `./gate.sh --quick` tests only what changed since its last green run (not since `main`), and
  both gates run the test binaries in parallel (`cargo xtask test-all` in the full gate).

### Fixed

- `cargo build --release` failed in `autoxide-plans` ("queries overflow the depth limit"); debug builds were fine.
- `POST /api/v1/auth/me/reauth` with a wrong authenticator code reported the error on field `code` instead of
  `totp_code`, the field that was sent.

- `GET /api/v1/auth/me` in a tenant entered through a delegation or a support grant answered `401`: it now returns
  the account and the permissions you have in that tenant, with `tenant_id`, `access` (`direct`, `delegation`,
  `support`) and `home_tenant_id`; `GET /api/v1/auth/me/memberships` returns an empty list there instead of `404`.
- A missing or ill-typed query parameter (e.g. `locale` of `GET /api/v1/content/public/posts`) answered `400
  bad_request` with the library's raw English text; it now answers `422 validation_failed` with the field in
  `errors`, translated like every other validation problem.
- `PATCH /api/v1/kernel/tenants/{id}` takes the tenant's slug as well as its id, like the other tenant endpoints.
- OpenAPI: `read_only` of tenants and the three fields of `TenantExpanded` are marked required (they are always
  present); the "Conventions" section explains how the tenant of a request is chosen (`tenant_required`,
  `tenant_not_found`, `tenant_mismatch`); descriptions of `comments.*` tenant settings no longer contain runs of
  spaces.
- Security: `GET /api/v1/auth/support-grants` and `DELETE /api/v1/auth/support-grants/{id}` checked only the
  permission, so the administrator of an ordinary tenant (who holds every permission) could list or revoke support
  grants of other tenants. They are now for platform staff in the platform tenant only, like granting
  (`403 auth.support_platform_only`).
- Adding a passkey failed on authenticators without the `credProtect` extension (the server required it): it is now
  asked for but not enforced. Signing in with a passkey still always verifies the user.
- `GET /api/v1/kernel/modules/{name}` answered 500 for modules with data upgrades (`auth`, `webhooks`).

- Lists of one tenant ordered by id (audit entries, users, roles, media, content entries, terms, authors, redirects,
  comments, bans, reports, forms, newsletter lists, subscribers, campaigns, suppressions, webhook endpoints, menus)
  have an index on `(tenant_id, id)`; without it SQLite read and sorted every row of the tenant (the audit list was
  about twice as slow after the multi-tenant change). For module authors: `tenancy::migration::list_index`.
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
