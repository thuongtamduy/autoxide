# Taxes

[Tiếng Việt](TAX.vi.md)

The `tax` module computes taxes from **data**: tax categories and rates with their effective dates,
withholding rates for platforms, and rules (such as when cash register invoices are required) come
in a tax data pack, not in the code. The server ships with a pack for Vietnam. Other modules (sales,
invoices, the ledger) compute taxes through `autoxide_tax::api::TaxEngine`.

> Every rate in the shipped pack carries its legal basis and the note "needs checking against the
> original document". Check them with someone qualified before using them in production.

## The Vietnam pack

| Category | Rate | Notes |
|---|---|---|
| `vat.standard` | 10% | Law on VAT 48/2024/QH15 |
| `vat.standard_reducible` | 8% until 2026-12-31, then 10% | the 2% reduction; goods excluded from it use `vat.standard` |
| `vat.reduced_5` | 5% | |
| `vat.zero` | 0% | exports |
| `vat.exempt` | not subject | shown as "not subject to VAT", not as 0% |
| `excise.beer` | 65% special consumption tax, then 10% VAT on the price including it | sample |

Withholding by marketplaces with payment (Decree 117/2025/ND-CP) for household businesses: goods 1% VAT
+ 0.5% PIT, services 5% + 2%, production and transport 3% + 1.5%. Businesses declare themselves:
nothing is withheld. What core does today: computes the amount (`POST /api/v1/tax/withholding`) and books it
when a settlement asks for it (`ledger`, entry `TaxWithheld`). What it does **not** do yet (with the
marketplace module, after 1.0): withhold by itself on every payout to a household seller, issue the
withholding certificate (`withholding_certificate` exists as a document kind, nothing creates it), and
prepare the data of the declaration on the seller's behalf — a marketplace does these itself until then.

The pack is imported by itself within an hour after the server starts (and whenever an update ships a
newer one). To import it at once, or another pack:

```bash
autoxide task tax.import_pack
autoxide task tax.import_pack path=/path/to/pack.json
```

or `POST /api/v1/tax/packs` (platform staff with `tax.packs_manage`). Only a higher `version` than the
imported one is accepted; the same pack again changes nothing. `autoxide doctor` warns when a rate ends
within `AUTOXIDE_TAX_EXPIRY_WARNING_DAYS` (60) days with no rate after it — for example if the 8% rate
is extended past 2026, import a pack that says so.

## Compute taxes

```bash
curl -X POST https://api.example.com/api/v1/tax/quote \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"jurisdiction": "VN", "tax_point": "2026-11-17", "price_mode": "inclusive", "currency": "VND",
       "lines": [{"line_id": "1", "category": "vat.standard_reducible", "amount": {"amount": "108000", "currency": "VND"}}]}'
```

- `price_mode`: `exclusive` (amounts before taxes) or `inclusive` (taxes are split out).
- Taxes are rounded **once per rate on the whole document**, then shared between the lines: the lines
  always add up to the totals. A later stage (VAT) is computed on the price plus earlier stages
  (special consumption tax).
- `tax_point` is a calendar day, not an instant. Orders and invoices compute it with `TaxEngine::tax_date`:
  the day in the time zone of the seller's tax jurisdiction (pack rule `tax_date.time_zone`; Vietnam:
  `Asia/Ho_Chi_Minh`), else in the unit's `organizations.timezone`, else in UTC. So an order placed at
  00:30 on 1 January 2027 Vietnam time (17:30 UTC on 31 December) is taxed at the rate of 1 January. No
  rate on that day: `422 tax.rate_missing` — the engine never guesses.
- **The order's tax is an estimate; the invoice decides.** The law ties the VAT tax point to the transfer of
  ownership of goods (delivery) or the completion of a service. An order taxes at the day it is placed; its
  invoice taxes at its own issue date (`orders.invoice_timing`, `on_delivered` by default). When a rate
  changes in between, the invoice uses the new rate and the order's documents
  (`GET …/orders/me/orders/{id}/documents`) show `differs_from_order: true` — settle the difference with the
  buyer. A draft invoice is re-taxed at issue
  when it was computed on another day. Adjustments and replacements keep the tax date (rate) of the invoice
  they correct. Ledger entries of orders and invoices are dated with the same tax date. `autoxide doctor`
  warns when an imported tax jurisdiction declares no `tax_date.time_zone`.
- The answer has a `snapshot` (rates and their ids, pack version, rounding): keep it with the document.

`POST /api/v1/tax/withholding` computes what a platform withholds from a payment to a seller.
MCP tools: `tax.quote`, `tax.rates`.

## Tax profiles

Each organization unit can have a tax profile — tax code (10 or 13 digits, `0101234567-001` for a
branch, or a 12-digit personal id), regime, business lines, estimated yearly revenue, legal name and
address: `PUT /api/v1/tax/profiles/{org_id}` (`tax.profiles_manage` on the unit). A unit without its own
profile uses the nearest one above it. The regime decides the invoice: `vat_credit` → VAT invoice;
`vat_direct`, `household` → sales invoice without VAT per line; `exempt` → no tax. A household business
with revenue from 1 billion VND a year must issue cash register invoices
(`TaxEngine::invoice_requirements`, Decree 70/2025/ND-CP — check the latest schedule).

## Read the data

`GET /api/v1/tax/jurisdictions`, `/categories`, `/rates?on=YYYY-MM-DD`, `/withholding-rates`, `/rules`,
`/packs` (`tax.read`).
