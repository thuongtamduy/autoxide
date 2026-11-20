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
| `GET /public/products/{id}/similar?locale=vi` | up to 12 products whose texts mean something close (AI embeddings); without embeddings, products of the same categories, best sellers first |
| `GET /public/products/{id}/frequently-bought-together?locale=vi` | up to 12 products found in the same completed orders over the last 90 days, most orders first |

Product cards carry `variant_summary` ("3 sizes"), `brand` (the attribute with code `brand`) and
`unit_price` (price per kg, l, m or piece) when a variant has a net content (`net_quantity` +
`net_unit`, e.g. 500 and `g`). Product pages give each variant a `path` with `?variant=<id>`,
`unit_price`, and for a temporary price `price_valid_until` and `regular_price` (the price just before
it) — what structured data (`Product`, `ProductGroup`, `Offer`) needs; the GTIN is `barcode`.

Search helpers:

- `GET /public/search/suggest?q=…&locale=vi` — suggestions as the shopper types (products,
  categories, and what other modules add), exact accents first, at most 10.
- `GET /public/products/facets` — takes the filters of `/public/products` and counts products per
  attribute value and per seller (the counts of one filter ignore that filter).
- `GET` / `PUT /synonyms` — the tenant's synonym groups (`[["áo thun", "áo phông", "tee"]]`); search
  and suggestions use them (`catalog.categories.manage`).

Best sellers and ratings come from module `orders`: each completed order (`orders.order_completed`)
counts its products once (a redelivered event is not counted twice), over a rolling 90-day window that
a daily job (`catalog.refresh_sales`) moves forward; review events carry the product's current review
count and star total. The same orders feed "frequently bought together". "Similar" needs the `ai`
module with embeddings on (`AUTOXIDE_EMBEDDING_PROVIDER` and the other `AUTOXIDE_EMBEDDING_*` settings): each product on sale gets one vector per
language, and its list of similar products is computed when it is indexed, not on each read.

Which products a website shows: those of its own unit, of the units above it, and of sellers below
it (`seller.own = false` on the card). A store hides a product of its chain with
`PUT /stores/{org_id}/overrides/{product_id}` `{"hidden": true}` (`catalog.products.write` at the store).

Prices on these pages are the current prices of the website's price list (the closest unit with an
active price list), without customer group. They are kept ready in a derived table rebuilt by a
background job after every change of price, status, price list or store choice, and every minute for
scheduled prices that just started or ended (event `catalog.price_effective`). If the public lists ever
look out of date, run `autoxide task catalog.rebuild_storefront [tenant=<slug>]`.

## Collections

A collection groups products for a page of the website ("Summer sale", "New this month"). Manage them
under `/collections` (`catalog.collections.manage` at the collection's unit; reading needs
`catalog.products.read`); `PATCH` and `PUT /collections/{id}/items` need `If-Match`.

- `kind: manual` — chosen products in order: `PUT /collections/{id}/items` `{"product_ids": [...]}` (at
  most 500) replaces the list.
- `kind: rule` — products matching every condition of `rules` (at most 10):
  `{"category_ids": [...], "attributes": {"color": ["red"]}, "price_min": 100000, "price_max": 500000,
  "new_within_days": 30, "on_promotion": true, "match": "all"}`. Prices are in the smallest unit of the
  website's price list currency; `on_promotion` needs module `promotions` (`catalog.rule_needs_module`).
- `sort`: `manual` (chosen order, manual collections only), `newest`, `price_asc`, `price_desc`,
  `best_selling`; equal values keep a stable order by id on both databases.
- `texts` by language: `title`, `slug` (unique in the language), `description`, `meta_title`,
  `meta_description`; `media_id` (a public picture); `status` (`draft`, `active`, `archived`) and an
  optional `starts_at` / `ends_at`.
- `ai_assisted`: the texts were written with an AI's suggestion; the public page carries it so the
  website shows an AI label (Vietnam's Law on Artificial Intelligence — check the original text). MCP
  tools set it by default; a person clears it with `PATCH` after rewriting the texts.

The website reads `GET /api/v1/catalog/public/collections/{slug}?locale=vi` (public; `site` or the
domain): the collection's texts and one page of its products, with the same cards as
`/public/products`, only products still shown on that website, in the collection's order. Only `active`
collections in their time window, of the website or a unit above it, are public. Banners and menus
point to a collection as `catalog.collection` of the object directory (`/collections/<slug>`). MCP tools:
`catalog.list_collections`, `catalog.save_collection` (drafts only: a person sets a collection `active`).
A worked home-page example: [`docs/guides/storefront.md`](guides/storefront.md).

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
emits `promotions.automatic_prices_changed` (`after.variant_ids`) when they change. A marketplace
module can refuse to put a product on sale with `set_listing_gate` (a `ListingGate`); the status
change then answers 422 `catalog.listing_denied`.

`catalog.display_price_changed` tells when the public price of a product on a website changes (the
automatic sale price when there is one, otherwise the lowest variant price; `null` when it is not on
sale there; never the crossed-out `compare_at`). A rebuild that leaves the price as it was sends
nothing.

## Import and export (CSV)

Both run in the background and need file storage (module `files`); without it they answer 503
`catalog.storage_unavailable`. Files are kept 7 days.

- `POST /imports?org_id=<unit>` with the file itself as the body (`Content-Type: text/csv`, UTF-8, at
  most 20 MiB and 50 000 rows; `catalog.import`). Columns, first line, any order: `sku` (required),
  `name`, `locale`, `short_description`, `description`, `kind`, `tax_category`, `barcode`,
  `weight_grams`. A known SKU updates that variant and the product's text in `locale` (the address
  stays); a new SKU creates a **draft** product with one variant (needs `name` and `tax_category`).
  Nothing is put on sale by an import, so importing the same file again changes nothing. Each row is
  checked like a manual change with your rights: a bad row is reported and the others are written.
- `GET /imports/{id}`: `status` is `queued`, `running`, `succeeded`, `succeeded_with_errors` (see
  `errors`: line, column, code, message), `rejected` (the file is not valid; nothing was written) or
  `failed`. Only the person who started it sees it. Products with several variants or options are
  completed through the API after the import.
- `POST /exports` `{"org_id": …}` (`catalog.export`): every variant of the products you can read, with
  the same columns, so the file can be edited and imported back. Cells a spreadsheet would run as a
  formula (`=`, `+`, `-`, `@`) start with `'`; an import removes that `'` again. When `GET
  /exports/{id}` says `succeeded`, download `content_path`.

MCP tools: `catalog.start_import`, `catalog.import_status`.

## AI drafts of descriptions and SEO

`POST /products/{id}/suggestions` `{"locale": "vi"}` (`catalog.products.write`, plan feature
`catalog.ai`, the `ai` module on) asks the model for a short description, a description and SEO texts
using only what the product's texts already say. The result is a **draft**: nothing on the product
changes. `POST /suggestions/{id}/apply` writes it into that language's text (name and address unchanged)
and marks the product `ai_assisted`; `POST /suggestions/{id}/discard` drops it. MCP tool:
`catalog.suggest_copy`.

## Product Q&A, menus and other modules

Products and categories are in the kernel's object directory, so other modules can point at them
without reading catalog tables:

- `catalog.product`: public when on sale (`active`) with a text in the language asked; title, short
  description, address `/<prefix>/<slug>` and first image. Product questions and answers use module
  `comments` with this target type. Suggested settings: `AUTOXIDE_COMMENTS_TARGET_TYPES` including
  `catalog.product`, `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES=catalog.product` (verified accounts only,
  Decree 147/2024 — check the scope with the original text) and
  `AUTOXIDE_COMMENTS_MODERATION_BY_TYPE=catalog.product:user=pre` (questions wait for review). Archiving
  or deleting a product closes and hides its questions. An author who can manage the product
  (`catalog.products.write` on its unit) gets the badge `seller`; module `orders` adds
  `verified_buyer`.
- `catalog.category`: for website menus (module `menus`) — names and addresses (`/categories/<slug>`)
  by language, children in display order, only visible categories. A "best sellers" block in a menu is
  the public product list filtered by category and sorted `best_selling`.

Image texts by language: `PUT /products/{id}/media/{media_id}/alts/{locale}` `{"alt": "…"}` (with
`If-Match`; `DELETE` removes it). Product pages in that language show it; otherwise the product's own
`alt` in its default language, then the media library's text in that language. Removing an image from
the product removes its texts.

## What differs between variants

`PUT /variants/{id}/attributes` sets attribute values on one variant — how many people it serves, what
is in the box, care instructions, dishwasher safe — using the tenant's attributes (`POST /attributes`),
so nothing is fixed to one trade. Text values may carry translations: `{"attribute_id": …, "value":
"Rửa tay", "texts": {"en": "Hand wash"}}`. The public product page lists them under each variant
(`variants[].attributes`) in the language asked.

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `AUTOXIDE_CATALOG_LOCALES` | `en,vi` | Languages product and category texts may be written in |
| `AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX` | `products` | First segment of a product's public address |
