# Money, ledger and accounting documents

[Tiếng Việt](MONEY.vi.md)

Three modules handle money in Autoxide:

| Module | What it does | Other modules use |
|---|---|---|
| `tax` | tax categories, rates with effective dates, tax profiles, the Vietnam data pack ([TAX.md](TAX.md)) | `dyn TaxEngine` |
| `ledger` | double-entry books, payments through licensed partners, money-flow modes, holds, payouts, VietQR transfers, exchange rates | `dyn Ledger` |
| `invoices` | document series, gapless numbering, e-invoices, print templates, accounting exports, archives | `dyn Invoices` |

**Autoxide never holds money itself.** There is no wallet and no balance "of the buyer" kept by the
platform: money goes from the buyer to a licensed payment partner or straight to the seller's bank
account. The ledger only records **obligations** (what is owed to whom) and checks them against the
partners' statements.

> Legal notes in this guide (accounting law, e-invoices, withholding, data protection) are a starting
> point, **not legal advice**: check them against the original documents with someone qualified before
> running a real business on them.

All routes are under `/api/v1/<module>`, described in `docs/openapi.json` (served at
`/api/v1/openapi.json`). Amounts are `{"amount": "1999", "currency": "USD"}` — an integer of the
currency's smallest unit, as a string. Rates and quantities are decimal strings such as `"25345.5"`,
never JSON numbers.

## The ledger

- **One book per organization unit** (the tenant's root, a subsidiary, a seller that keeps its own
  accounts), in a book currency (default: the unit's currency, else VND).
- **Accounts are named by purpose**: `cash.bank`, `receivable.customer`, `revenue.sales`,
  `tax.vat_output`, `payable.seller_held`… plus a counterparty (`org`, `customer`, `carrier`,
  `partner`, `supplier`) when needed. Other modules add purposes with their own prefix.
- **Entries are double-entry and append-only**: debits equal credits, every line is positive, all
  lines in one currency. A mistake is corrected with a reversing entry (`POST
  /ledger/entries/{id}/reverse`); an entry can be reversed once.
- **Every write takes a `reference`**: sending the same reference with the same content returns the
  first result (200); with other content it is refused (`reference_reused`).
- **Monthly periods** can be closed (`POST /ledger/periods/{id}/close`); a closed period takes no
  entry — post an adjustment in an open period. Closed periods do not reopen.
- **Tamper evidence**: entries are sealed into a hash chain within seconds; `autoxide doctor` reports
  the first entry changed or removed outside the module (`ledger.chain`) and any balance that does not
  match its lines (`ledger.balances`).

Reading: `GET /ledger/entries` (filter by `source_module` and `source_id` to see the entries of a
document), `GET /ledger/accounts/{id}/balance?as_of=`, `GET /ledger/trial-balance`.

## Payments, flow modes, holds and payouts

A payment is created with `POST /ledger/payments` and paid through a **payment partner** (a connector
module that implements the kernel trait `PaymentGateway`). The partner's webhook
(`POST /ledger/webhooks/{provider}`, signature checked on the raw body) marks it paid; a daily
reconciliation compares the ledger with the partner's statement and puts every difference in a queue
for a person (`GET /ledger/exceptions`).

Each unit has a **money-flow mode** (`PUT /ledger/flow-settings/{org_id}`, inherited down the tree):

| Mode | Who receives the money | What the ledger records |
|---|---|---|
| `self_collect` (default) | the seller, directly | the sale and VAT in the seller's book |
| `partner_collect` | a licensed partner collects for the marketplace | what is owed to the seller (`payable.seller_held`), held until the return period ends, then available for payout |
| `split_at_gateway` | the partner splits the payment at once | only the marketplace's share |

Holds are released by a job (every 5 minutes) or by hand (`POST /ledger/holds/{id}/release`). Payouts
(`POST /ledger/payouts`) go through a `PayoutGateway` partner and never exceed the available balance,
even when sent at the same time. Platform fees carry their own VAT; withholding for household
businesses (Decree 117/2025/ND-CP — needs checking) only happens in the collect modes; core computes and
books it, but does not yet withhold on each payout by itself, issue the withholding certificate or prepare
the declaration (with the marketplace module, after 1.0 — see `TAX.md`).

## VietQR bank transfers

`POST /ledger/payments/vietqr` returns a VietQR payload (NAPAS / EMVCo format) whose transfer note is a
short code. Money goes **straight to the unit's bank account**. A bank feed (`BankFeed`: a bank or a
statement-reading service, by webhook or by polling) reports incoming transfers; a transfer with the
right code and amount marks the payment paid, anything else (wrong amount, no code, paid twice, late)
goes to the exceptions queue. Only a fake bank feed ships today: a real connector needs a contract
with the bank or service.

### A payment in another currency

A payment of an order that the ledger reports as received in another currency than the order (for example a
foreign-currency transfer accepted while resolving an exception) is **not** counted towards the order: `orders`
never records units of one currency as another. The order stays waiting for payment (and expires as usual), the
error log line names `order_id` and `payment_id`, the counter `orders_payment_currency_mismatch_total` goes up (once per delivery attempt of the event, so one
payment can count several times) and
the alert `AutoxidePaymentCurrencyMismatch` fires. The event `ledger.payment_succeeded` keeps the error code
`orders.payment_received_currency_mismatch` and goes to the dead-letter queue after its last retry (delivering it
again cannot change the currency). To handle it: refund the payment (`POST /api/v1/ledger/payments/{id}/refunds`),
then ask the buyer to pay again in the order's currency, or cancel the order.

## Accounting documents

`invoices` keeps documents of these kinds: `vat_invoice`, `sales_invoice`, `cash_register_invoice`,
`retail_receipt`, `delivery_note`, `goods_receipt`, `cash_receipt`, `cash_payment`,
`withholding_certificate`.

- A **draft** has no number; taxes are computed from the tax categories of its lines.
- **Issuing** (`POST /invoices/documents/{id}/issue`) gives the next number of the series with **no
  gaps**, even under concurrent issues; issuing again returns the same number. Sales documents are
  booked in the seller's ledger (receivable, revenue, output VAT).
- **E-invoices** go to a provider (`EInvoiceProvider`, including invoices from cash registers) in a
  background job; a refused invoice is fixed and sent again with the same number. Issued documents
  are corrected only by an adjustment or a replacement, or cancelled. An e-invoice already sent to the tax
  authority cannot be cancelled where the seller's tax jurisdiction forbids it (Vietnam since 2025-06-01,
  Decree 70/2025/ND-CP; tax pack rule `einvoice.cancel_after_submission`): `409
  invoices.einvoice_not_cancellable` with the legal basis — issue an adjustment (`direction: decrease`) or a
  replacement. One the tax authority rejected is cancelled locally. Autoxide does not send the error notice
  form 04/SS; lodge it at the provider's or tax authority's portal when it is required.
- A unit may issue for a seller under a **mandate** (`/invoices/mandates`).

## Exchange rates and foreign-currency documents

Rates live in the ledger, per tenant, per currency pair, with an **effective date**: a rate is used
from its day until a later rate of the same pair.

- Record by hand: `POST /ledger/exchange-rates` `{"base": "USD", "quote": "VND", "rate": "25345.5",
  "effective_date": "2026-11-17"}` (permission `ledger.settings_manage` for the whole tenant).
  Recorded rates are never changed; a different rate for the same pair and day is refused.
- Fetch from a rate source: `POST /ledger/exchange-rates/fetch` (a connector implementing the kernel
  trait `ExchangeRateSource`; only a fake source ships today).
- Look up: `GET /ledger/exchange-rates/effective?base=USD&quote=VND&on=2026-11-17` — the latest rate on
  or before that day; on the same day a hand-entered rate wins over a fetched one. MCP tool
  `ledger.exchange_rate`.

A **foreign-currency document** takes the rate of its issue date when it is issued and keeps a copy
(`exchange_rate`, `exchange_rate_id`, `base_total`); later rates never change it. Without a rate,
issuing fails with `invoices.rate_missing`. Entries in a currency other than the book's store the rate
on every line and an amount in the book currency; the converted total is rounded once and shared
between the lines, so the entry balances in both currencies. A reversal uses the rate of the entry it
reverses. (Accounting Law 88/2015, Article 11: amounts in foreign currency are converted to đồng —
needs checking.)

## Printing documents

`GET /invoices/documents/{id}/render?format=html|pdf&locale=vi|en` prints a document.

- **HTML** comes from a **print template**: the unit's template for the document kind and language,
  else the tenant's, else the built-in one. Templates use Jinja syntax (`{{ buyer.name }}`,
  `{% for line in lines %}`, filters `money`, `number`, `date`) and run in a **sandbox**: no files,
  no network, no other templates, limits on size (`AUTOXIDE_TEMPLATE_MAX_BYTES`), steps and output.
  Every value is HTML-escaped and there is no `safe` filter.
- Manage templates with `/invoices/templates` (permission `invoices.templates_manage`). Saving checks
  the template and reports every problem at once with line and column: `template.syntax_error`,
  `template.unknown_variable`, `template.required_variable_missing`. `GET /invoices/template-defaults`
  gives the built-in template and the variables; `POST /invoices/templates/preview` tries a template
  without saving it.
- **PDF** has a fixed A4 layout built from the same data (title, parties, lines with wrapping and page
  breaks, totals, signature blocks), with an embedded font that carries Vietnamese marks. Templates
  change the HTML only; for a PDF in your own layout, print the HTML from a browser.
- Other modules (orders, inventory) print their delivery notes, goods receipts and receipts the same
  way: they create the document through `dyn Invoices` and call `Invoices::render`.

## Accounting exports

`POST /invoices/exports` `{"profile": "generic_journal", "format": "csv", "from": "2026-01-01",
"to": "2026-01-31"}` (permission `invoices.export`) makes a file in the background; follow the
`Location` until `status` is `ready`, then `GET /invoices/exports/{id}/download`.

| Profile | Rows |
|---|---|
| `generic_journal` | one row per entry line: posting date, entry, document date and number, description, account, debit, credit, currency, rate, amounts in the book currency, counterparty |
| `vn_double_entry` | debit/credit pairs (`TK Nợ`, `TK Có`, `Số tiền`) — the shape Vietnamese accounting software imports as general vouchers through its column-mapping step |
| `generic_documents` | one row per document line: date, series, number, buyer, item, quantity, price, VAT rate and amounts |

- CSV is UTF-8 with a BOM (opens in Excel with Vietnamese marks), cells starting with `= + - @` are
  prefixed with `'` (no formula injection). XLSX is also available.
- Account numbers: the account's own `external_code`, else the Vietnamese chart of accounts mapping
  of its purpose (`modules/ledger/data/vn-accounts.json`, e.g. `revenue.sales` → 5111 — **needs
  checking by an accountant** against Circulars 99/2025/TT-BTC and 133/2016/TT-BTC).
- The vendor templates of MISA, Fast and other packages are downloaded inside each product and change
  between versions, so no vendor-specific profile ships yet; map the columns of `vn_double_entry` in
  the product's import screen.

## Keeping records for 10 years

Accounting documents and books are kept at least 10 years after the end of the accounting year
(Accounting Law 88/2015/QH13 Article 41; Decree 174/2016/ND-CP Article 12 — needs checking).

- Issued documents cannot be deleted (`invoices.retention_hold`); each carries `retention_until`.
- Deleting a tenant for good keeps the accounting tables (`ledger_*`, `invoices_series`,
  `invoices_documents`, `invoices_lines`, `invoices_mandates`, `invoices_archives`); everything else is
  removed.
- **Archive copy**: `POST /invoices/archives` (permission `invoices.archive`) writes the general journal
  (`ledger/journal.csv`), every document (`documents.jsonl`) and `manifest.json` (SHA-256 of every file)
  to the shared file storage under `archives/<tenant>/<id>/` — outside the tenant's folder, so it stays
  after the tenant is deleted. A deleted tenant without an archive gets one automatically.
- **After the retention period** a daily job (`invoices.purge_expired`) removes the personal data of
  consumer buyers from expired documents (name, email, address, personal id); for deleted tenants it
  deletes expired documents and drafts. Documents still inside their retention period are never
  touched. The ledger of a deleted tenant is kept whole: its hash chain cannot be cut in part.

## Writing a partner connector

A connector is a module of its own that depends only on the kernel and registers **before**
`ledger`/`invoices`: it implements one of the kernel traits and contributes it in `start` with
`context.contribute::<dyn PaymentGateway>(Arc::new(…))` (or `PayoutGateway`, `BankFeed`,
`ExchangeRateSource`, `EInvoiceProvider`). Every call receives the tenant: read that tenant's
credentials from the tenant secret store, never from environment variables. Check webhook signatures
on the raw body with a constant-time comparison. Test with the fakes of `autoxide-testkit`
(`FakePaymentGateway`, `FakePayoutGateway`, `FakeBankFeed`, `FakeExchangeRateSource`,
`FakeEInvoiceProvider`).
