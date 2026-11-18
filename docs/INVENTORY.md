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
