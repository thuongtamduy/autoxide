# Catalog

[Tiếng Việt](CATALOG.vi.md)

The `catalog` module holds the products every commerce module reads: products with variants, a
category tree, and price lists with a full price history. Inventory, promotions, shipping, orders and
sales channels read products and prices through `autoxide_catalog::api::Catalog`, never through the
`catalog_…` tables. Everything is under `/api/v1/catalog`; every endpoint is described in
`docs/openapi.json`, and the same operations are MCP tools (`catalog.*`).

## Products and variants

- `POST /products` creates a product as a **draft** together with its first variant. `kind` is
  `physical` (shipped, stocked), `digital`, `service` or `bundle`. `tax_category` is a code of the tax
  engine (`GET /api/v1/tax/categories`), e.g. `vat.standard`.
- Texts are per language (`PUT /products/{id}/translations/{locale}`); the languages allowed are
  `AUTOXIDE_CATALOG_LOCALES`. Each translation has its public `path`,
  `/<AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX>/<slug>`. An empty `slug` is made from the name, with `-2`, `-3`… when taken. The
  description is plain text (blank lines separate paragraphs); HTML is not interpreted.
- Options (colour, size…) are set with `PUT /products/{id}/options`: up to 3 options of up to 100
  values. Adding an option gives every existing variant the option's first value. Each variant holds
  one value of each option, and no two variants share a combination
  (`catalog.option_combination_taken`).
- SKUs and barcodes are unique in a tenant (`catalog.sku_taken`, `catalog.barcode_taken`); SKUs are
  compared ignoring case and spaces. A numeric barcode of 8, 12, 13 or 14 digits must have a right GTIN
  check digit.
- Sale mode per variant: `in_stock`, `backorder` or `preorder` (with `preorder_available_on` after
  today and an optional `preorder_limit`).
- Images are public media of the `media` module (`PUT /products/{id}/media`).
- Status (`POST /products/{id}/status`, permission `catalog.products.publish`): `draft` → `active` or
  `archived`; `active` ↔ `inactive`; any → `archived`; `archived` → `draft`. Going on sale needs a tax
  category, a name and an active variant. A product that was on sale once cannot be deleted, only
  archived.
- Bundles (`kind: bundle`) list their components with `PUT /variants/{id}/components`; a bundle
  cannot contain a bundle, itself or a service.

The number of products is the plan limit `catalog.products` (archived products count). Plan features:
`catalog.variants` (more than one variant), `catalog.price_lists` (more than one price list),
`catalog.bundles`, `catalog.digital`, `catalog.ai`.

## Categories

One tree per tenant, at most 7 levels deep (`/categories`, permission `catalog.categories.manage`).
`POST /categories/{id}/move` moves a category with everything below it; moving it under itself is
refused (`catalog.category_cycle`). Filtering products by a category includes its subcategories.

## Attributes, restricted goods and readiness

- **Attributes** (`/attributes`, permission `catalog.categories.manage`) are typed: `text`, `integer`,
  `decimal` (with `scale`), `boolean`, `date`, `option`, `options`. Products set their values with
  `PUT /products/{id}/attributes`.
- A category lists its attributes with `PUT /categories/{id}/attributes`; an attribute can be
  **required**, from `effective_from` until `effective_to`, for the category and (by default) its
  subcategories. Required attributes are data: new labelling rules are added with their start date.
- **Prohibited and conditional goods** are shared rules with their legal basis
  (`GET /restriction-rules`). The operator loads them with
  `autoxide task catalog.load_restrictions` (the `vn` pack ships with the server; the same command
  creates sample product-label attributes such as origin and use-by date for a tenant). Link rules to a
  category with `PUT /categories/{id}/restrictions`. The rules were compiled from secondary sources —
  check them against the source texts with a legal professional.
- A conditional product needs valid documents (`/products/{id}/documents`, permission
  `catalog.documents.manage`), e.g. an alcohol retail licence.
- Every product has a **readiness**: `restricted` (prohibited goods), `missing_attributes`,
  `missing_documents` or `ready`. Only `ready` products go on sale. Filter with `?readiness=` and count
  with `GET /products/counts?group_by=readiness`. Expired documents are noticed by a daily job.
- Going on sale also checks that the tax category exists in the tax engine for the unit's tax
  profile (`catalog.tax_category_unknown`).

## Prices

A **price list** belongs to an organization unit (a chain or a store) and applies there and below.
It has a currency, `tax_inclusive`, an audience (`consumer` or `business`), an optional customer
group and a `priority`.

**Price rows are only ever added.** A new price is a new row with its own `valid_from`; a future
`valid_from` schedules a price, and `valid_to` makes it temporary — when it ends, the earlier price
applies again. The history is `GET /price-lists/{id}/prices`. `POST /price-lists/{id}/prices` adds up
to 500 rows under your `reference`: sending the same reference and content again returns the rows
already written.

**Which list wins.** For a store, customer group and time, the lists that apply are the active lists
in the currency, at the store or a unit above it, whose customer group (and company, channel, market)
is empty or matches. They are ordered: the unit closest to the store first, then a matching company,
customer group, channel and market, then higher `priority`, then id. Each variant takes its price from
the first list in that order that has one. `GET /variants/{id}/price?org_id=…&currency=…` shows the
price and the ordered candidate lists.

**Consumer prices in Viet Nam** must be in VND and include tax when the unit has a Vietnamese tax
profile (Price Law 16/2023/QH15 — check the source text with a legal professional before relying on
it).

**Derived price lists** take their prices from a source list (`derived_from`; one level only,
`catalog.derived_chain`), adjusted by `adjust_bp` (basis points, signed: `-1000` = 10% off) and
`adjust_minor`, then rounded by a **rounding rule** (`/rounding-rules`: multiples of `step_minor`, or
`k × step_minor + ending_minor`, e.g. step 10,000 and ending 9,000 give 129,000 and 199,000). A job
writes real price rows into the derived list whenever the source gets prices, so reading a price
never computes a formula. A manual row in the derived list pins that variant's price while it is in
effect.

**Bulk price changes** (`/price-lists/{id}/changes`): create (`set`, `percent` or `amount`, for
categories, products or variants, optionally temporary with `valid_to`) → preview (counts, 20
sample rows, price-floor violations; nothing is written) → approve (`catalog.prices.approve`; with the
unit setting `catalog.price_change_requires_second_person` on, someone other than the maker must
approve) → a job writes the rows (`source = bulk`). Undo a change with a new change back to the old
prices.

**Price floors** (`/price-guards`): a minimum price and / or a minimum margin over unit cost, per unit
and optionally per category, acting as `warn` (the price is saved with a warning) or `block`
(`catalog.price_below_floor`). The rule of the closest unit wins; within a unit, the rule of the
closest category wins. Unit costs come from the `inventory.cost_changed` event.

## Public reads for websites

No sign-in needed (`/api/v1/catalog/public/…`). Only products that are `active`, have a price on the
website asking and a text in the language asked; the website comes from the domain or `?site=<unit>`.
Every answer has an `ETag` (send it back in `If-None-Match` to get 304) and
`Cache-Control: public, no-cache`.

| Endpoint | What |
| --- | --- |
| `GET /public/products?locale=vi` | product cards; filters `category=<slug>` (with sub-categories), `attr.<code>=<value>[,<value>]` (filterable attributes), `price_min` / `price_max`, `q` (with or without accents), `on_sale`, `seller=<unit>`, `sold_by=own\|marketplace`, `min_rating`; `sort=newest\|price_asc\|price_desc\|best_selling\|rating` |
| `GET /public/products/by-slug/{slug}?locale=vi` | the product page: variants with current prices, images, attributes, categories, other languages |
| `GET /public/categories?locale=vi` · `GET /public/categories/{slug}?locale=vi` | visible categories (a category under a hidden one is hidden) |
| `GET /public/sitemap` | every product address in every language, at most 200 per page |

Search helpers:

- `GET /public/search/suggest?q=…&locale=vi` — suggestions as the shopper types (products,
  categories, and what other modules add), exact accents first, at most 10.
- `GET /public/products/facets` — takes the filters of `/public/products` and counts products per
  attribute value and per seller (the counts of one filter ignore that filter).
- `GET` / `PUT /synonyms` — the tenant's synonym groups (`[["áo thun", "áo phông", "tee"]]`); search
  and suggestions use them (`catalog.categories.manage`).

Which products a website shows: those of its own unit, of the units above it, and of sellers below
it (`seller.own = false` on the card). A store hides a product of its chain with
`PUT /stores/{org_id}/overrides/{product_id}` `{"hidden": true}` (`catalog.products.write` at the store).

Prices on these pages are the current prices of the website's price list (the closest unit with an
active price list), without customer group. They are kept ready in a derived table rebuilt by a
background job after every change of price, status, price list or store choice, and every minute for
scheduled prices that just started or ended (event `catalog.price_effective`). If the public lists ever
look out of date, run `autoxide task catalog.rebuild_storefront [tenant=<slug>]`.

## For other modules

```rust
let catalog = context.require::<dyn autoxide_catalog::api::Catalog>()?;
```

`variants`, `variant_by_code`, `prices` (price in effect at a time), `reference_prices` (the price in
effect just before a time — for "price before the discount"), `price_floors` and `bundle_components`.
Every call takes at most 500 ids and runs a fixed number of queries. Events are listed in
`autoxide_catalog::api::events`.

A module registered after `catalog` plugs in through `dyn CatalogExtensions`: a promotions module gives
the public lists their automatic sale prices with `set_sale_price_source` (a `SalePriceSource`), and
emits `promotions.automatic_prices_changed` (`after.variant_ids`) when they change.

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `AUTOXIDE_CATALOG_LOCALES` | `en,vi` | Languages product and category texts may be written in |
| `AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX` | `products` | First segment of a product's public address |
