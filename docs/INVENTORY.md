# Inventory (`inventory`)

[Tiếng Việt](INVENTORY.vi.md)

Module `inventory` keeps the stock of every product variant in every warehouse or store of a tenant. Every change
goes into an append-only movement ledger, and stock is never oversold, even when many requests arrive at the same
time, on SQLite and on PostgreSQL. Other modules use it through `autoxide_inventory::api`; the HTTP API lives under
`/api/v1/inventory` (see `docs/openapi.json`) and the MCP tools are named `inventory.*`.

## Stock model

- A **warehouse** is an organization unit of type `warehouse` or `store` (step 34), or the tenant's root unit (a
  single shop keeps its stock there). A unit of another type answers `inventory.not_a_warehouse`.
- A **stock level** is one variant in one warehouse. It holds two numbers:
  - `on_hand`: units physically in the warehouse — always the sum of the level's movement ledger, never below 0;
  - `available`: units that can still be sold — `on_hand` minus the units reserved for orders. It may go below 0
    when goods leave a warehouse whose stock is all reserved (e.g. damaged) or when a product may be sold out of
    stock.
  `reserved` (`on_hand − available`) is computed, not stored.
- The **movement ledger** (`GET /movements`) only grows: `receipt`, `sale`, `return`, `adjustment`, `count`,
  `transfer_out`, `transfer_in`, `expiry`, `supplier_return`. A mistake is corrected with a new adjustment.
- Stock numbers change only through single conditional `UPDATE` statements (`rows::adjust`), never read-then-write.

## Adjusting stock

`POST /adjustments` (`inventory.adjust`) changes stock in one warehouse:

```json
{
  "org_id": "…warehouse…",
  "reference": "ADJ-2026-0042",
  "lines": [
    {"variant_id": "…", "quantity": -2, "reason_code": "damaged", "note": "Dropped"},
    {"variant_id": "…", "quantity": 50, "reason_code": "found",
     "unit_cost": {"amount": "120000", "currency": "VND"}}
  ]
}
```

- Every line needs a reason from the tenant's reason list `inventory.adjustment`
  (`GET /api/v1/kernel/reasons/inventory.adjustment`); built-in codes: `damaged`, `lost`, `found`,
  `data_entry_fix`, `sample_use`, `expired`, `count_variance`. Tenants rename, reorder, turn off and add codes.
- `reference` makes the call safe to retry: the same reference and content returns the adjustment already made
  (`200`); the same reference with other content is `reference_reused`.
- `on_hand` never goes below 0 (`inventory.insufficient_stock`).
- A positive line may carry a `unit_cost` (e.g. opening stock); it updates the average cost.

## Average cost

Each stock level keeps a weighted average unit cost for gross-margin reports: when goods with a cost come in,
`(on hand × old cost + quantity × unit cost) / (on hand + quantity)`, rounded half away from zero to the currency's
minor unit. With nothing on hand the cost is the incoming cost. Goods without a cost (returns, found items) keep the
cost. A change sends `inventory.cost_changed` (`catalog` uses it for price floors).

## Reorder points and alerts

`PATCH /levels/{id}` (`inventory.settings.manage`) sets `reorder_point`. `inventory.low_stock` fires once when
`available` falls from above the reorder point to it or below; `inventory.out_of_stock` when it falls from above 0
to 0 or below. `GET /levels?below_reorder=true` and the MCP tool `inventory.list_low_stock` list them.

## Reservations (for orders)

Module `orders` holds stock through `autoxide_inventory::api::Inventory`:

1. `reserve` — all lines of an order or nothing, until `expires_at` (at most
   `AUTOXIDE_INVENTORY_MAX_HOLD_HOURS`). Calling again with the same `order_id` and content returns the same
   reservation; other content is `reference_reused`. Bundles take stock from each component; digital goods and services
   are skipped. Variants sold as `backorder` or `preorder` may take `available` below 0 (`backordered`).
2. `confirm` — the order was placed: the reservation no longer expires.
3. `fulfil` — ship some or all reserved units (`fulfilment_key` makes it safe to retry); writes `sale` lines.
   With module `orders`, stock leaves when the goods reach the buyer: `inventory` listens to
   `orders.fulfillment_delivered` (one shipment; key `fulfillment:<id>`) and `orders.order_delivered` (the units no
   shipment carried, `after.delivered_lines`; key `order:<id>:delivered`), so `on_hand` and `reserved` drop and the
   order becomes `fulfilled`. Units on the way stay reserved (already out of `available`). Each line issues at most
   what is still held, so events arriving twice, late or in any order never issue more; an event for a reservation
   already `fulfilled` is done. Because the goods are already gone, this path may take units from an expired lot
   when the lots still in date are not enough (logged as a warning); a manual `fulfil` never does.
   Serial-tracked goods issue with the serial numbers recorded on the order line
   (`POST /api/v1/orders/{id}/lines/{line_id}/serials`) that are still in stock; units without a recorded serial
   stay reserved (logged as a warning) while the other lines are issued, and recording the serials later
   (`orders.line_serials_recorded`) issues them. Limitation: when one line is reserved in several warehouses,
   serials are matched warehouse by warehouse in order; a serial held in another warehouse makes that issue fail
   (logged) and the line stays reserved — issue it yourself through `fulfil`.
4. `release` — give back what was not shipped (cancelled, payment failed…). Releasing twice adds back once.
5. `restock` — returned goods come back in (`reference` such as `return:<id>`); unsellable lines are only recorded.

Unconfirmed reservations past `expires_at` are released by the job `inventory.expire_reservations` (every minute).
Each reservation takes stock with one conditional `UPDATE` per line, in a fixed order, so 100 buyers of the last
unit get exactly one success.

### Pre-orders

A variant sold as `preorder` may be reserved before stock exists. When the variant has a `preorder_limit` (set in
`catalog`), held and confirmed pre-orders never exceed it, even when many buyers order at the same moment: the extra
ones get 409 `inventory.preorder_sold_out`. Cancelling or expiring a pre-order gives its places back.

When stock arrives at a warehouse that has waiting pre-orders (receipt, transfer, return, positive adjustment), the job
`inventory.allocate_preorders` gives the free units to pre-order lines **in the order they were placed** and publishes
`inventory.preorder_allocated` for each line (`quantity` this time, `allocated_quantity` in total), so `orders` can
move the order on. Running the job again never allocates twice.

Admins see reservations with `GET /reservations` (`inventory.reservations.read`) and release a stuck one with
`POST /reservations/{id}/release` (`inventory.adjust`; recorded with their user id).

Module `inventory` also listens to `orders.order_confirmed`, `orders.order_cancelled` and `orders.return_received`
(when module `orders` is installed) and runs the same functions, so an event arriving before or after the direct call,
or twice, changes stock once. It also listens to `orders.preorder_cancelled`: only the cancelled pre-order lines of an
order that stays open are released (their stock and pre-order places come back; the rest of the order keeps its
reservation), once per event even when it is delivered again.

## Choosing warehouses

`PUT /sources/{store}` lists the warehouses serving a store or channel, by `priority`; without rows the store sells
its own stock. `PUT /settings/{store}`:

- `prefer_nearby` — warehouses in the buyer's province first (the province comes from the units' public profile in
  module `organizations`);
- `allow_split` — split an order over several warehouses when none has every line (default off: one order, one
  warehouse).

A reservation tries one warehouse that has every line first; with no candidate at all it answers
`inventory.no_source`, with not enough stock `inventory.insufficient_stock` naming each line.

## Transfers

`POST /transfers` (`inventory.transfer` at the source) creates a draft with a number such as `CK-2026-000001`.
`POST /transfers/{id}/ship` takes the units from the source (only stock not reserved for orders) and adds them to
`incoming` at the destination; `POST /transfers/{id}/receive` (at the destination) receives all or part — the
destination's average cost uses the source's cost at shipping. `close` writes off what never arrived; `cancel` works
on drafts. MCP tools `inventory.create_transfer`, `inventory.receive_transfer`.

## Stock counts

`POST /counts` (`inventory.count`) opens a count (`KK-…`) for one warehouse; `PUT /counts/{id}/lines` or
`POST /counts/{id}/import` (CSV with `sku`, `counted`, optional `reason_code`) records counted quantities. Each line
keeps the on-hand quantity at the moment it was counted, so selling goes on while counting. `POST /counts/{id}/apply`
(`inventory.adjust`) writes each difference `counted − expected_at_count` as a `count` ledger line with its reason
(default `count_variance`).

## Lots, expiry dates and serial numbers

Turned on per product in `catalog` (`lot_tracked`, `serial_tracked`).

- Adding units of a lot-tracked variant needs `lot_code` (and `expires_on`). Reservations hold stock at the level;
  the lot is chosen when goods leave: the named lot, or else the lots that expire first, then units without a lot —
  never an expired lot (`inventory.lot_unavailable`).
- `GET /lots?expiry_state=ok|expiring|expired`: `expiring` means within the unit setting `inventory.expiring_days`
  (default 30). A daily job writes expired lots off the stock (`expiry` ledger lines, event `inventory.lot_expired`).
- Serial-tracked variants need exactly one serial number per unit when added, shipped (`FulfilLine::serials`),
  adjusted down or transferred; a serial number can be sold only once, even by two shipments at the same time.
- Transfers carry lots and serial numbers: the destination gets the same lot codes and expiry dates.

## Purchasing

- `POST /suppliers` (`inventory.purchasing.manage`): suppliers belong to a purchasing unit; `GET /suppliers` shows
  `payable` — what that unit owes each supplier, from module `ledger` (absent without it).
- Purchase orders (`PO-…`): draft → `POST /purchase-orders/{id}/order` (units count as `incoming`) → goods receipts,
  in parts → `close` (what never arrived leaves `incoming`) or `cancel`.
- `POST /receipts` (`inventory.receive`, `PN-…`): against a purchase order (never more than ordered) or directly; each
  line updates the average cost and may carry a lot and serial numbers. With a supplier and module `ledger`, a job posts
  Debit `inventory.stock` / Credit `payable.supplier` once (`ledger_status`: `pending`, `posted`, `failed` — retry
  with `POST /receipts/{id}/retry-ledger` — or `not_applicable`). MCP tool `inventory.create_receipt`.
- `POST /supplier-returns` (`TH-…`): goods back to the supplier at the current average cost (or a given price); the
  posting is reversed.

## Public availability

`GET /api/v1/inventory/public/availability?variant_ids=<id>,<id>&store_id=<unit>` needs no sign-in. For each variant
(1 to 100 per call) and store it answers a level:

| Level | Meaning |
|---|---|
| `in_stock` | Enough to sell; the number is never shown |
| `low` | Fewer than `AUTOXIDE_INVENTORY_PUBLIC_LOW_THRESHOLD` (5) left; `quantity` says how many |
| `out` | None to sell now |
| `preorder` | Not in stock yet, sold as `preorder` |

A store sells from its warehouse sources (or its own stock): with `allow_split` the sources add up, otherwise the
fullest one counts, as for a reservation. Bundles count the sets their components make; variants not tracking stock
are always `in_stock`. Unknown or hidden variants are left out. Without `store_id` every store offering pickup is
listed. The answer changes as soon as stock is reserved; it carries an `ETag` (send it in `If-None-Match` for 304)
and `Cache-Control: public, max-age=5`. Module `catalog` filters "available at store X" with the same rule
(`Inventory::in_stock_at`).

## Back-in-stock alerts

Shoppers ask to be told when an item is back at a store (needs module `notifications`):

1. `POST /api/v1/inventory/public/restock-alerts` with `variant_id`, `store_id` (omit for the online shop), `email` or
   `phone`, and `consent: true` (plus the `consent_text` shown, kept as proof). Visitors who are not signed in send a
   `form_token` from `GET /api/v1/antispam/token?scope=inventory:restock_alerts`. A signed-in buyer may give no
   contact: alerts go to their account at once. The answer is always the same `202`.
2. Visitors get a message with a link to your page `AUTOXIDE_INVENTORY_RESTOCK_CONFIRM_URL` (`{token}` is replaced);
   the page sends the token to `POST /api/v1/inventory/public/restock-alerts/confirm`.
3. When the store can sell the item again, each confirmed alert gets **one** message (`inventory.back_in_stock`) per
   time the item comes back, never twice for the same wave. Every message links to the page again; sending its token
   to `POST /api/v1/inventory/public/restock-alerts/stop` stops the alerts.

The messages are transactional (asked for and confirmed by the person, no advertising). Module `customers` asks
`Inventory::restock_alerts_of` so a buyer is not told twice about the same item.

## Checking the numbers

`autoxide task inventory.verify_levels [tenant=<slug>] [org_id=<unit>]` compares every level with its ledger and
its reservations and only reports. `autoxide doctor` runs the same check on a sample of 1,000 levels per tenant.

## Permissions

| Permission | Allows |
|---|---|
| `inventory.read` | Stock levels, ledger, lots, valuation |
| `inventory.adjust` | Adjustments, applying counts, releasing stuck reservations |
| `inventory.receive` | Goods receipts |
| `inventory.transfer` | Shipping and receiving transfers |
| `inventory.count` | Stock counts |
| `inventory.reservations.read` | Reservations of orders |
| `inventory.settings.manage` | Reorder points, warehouse sources, store settings |
| `inventory.purchasing.read` / `.manage` | Suppliers, purchase orders, supplier returns |

All of them are scoped by organization unit: a person with a `unit` grant on warehouse A sees and changes only
warehouse A.

## Plans

Limit `inventory.warehouses` (units holding stock); features `inventory.multi_warehouse`, `inventory.transfers`,
`inventory.lots`, `inventory.serials`, `inventory.purchasing`.

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `AUTOXIDE_INVENTORY_MAX_HOLD_HOURS` | `72` | Longest reservation of an unpaid order |
| `AUTOXIDE_INVENTORY_EXPIRE_BATCH` | `200` | Expired reservations released per job run |
| `AUTOXIDE_INVENTORY_MAX_LINES_PER_RESERVATION` | `200` | Lines per reservation |
| `AUTOXIDE_INVENTORY_PUBLIC_LOW_THRESHOLD` | `5` | Below it the public availability says `low` |
| `AUTOXIDE_INVENTORY_RESTOCK_CONFIRM_URL` | — | Website page for back-in-stock alert links, with `{token}` |
