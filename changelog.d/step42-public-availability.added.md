- Module `inventory`: public availability `GET /api/v1/inventory/public/availability?variant_ids&store_id` — `in_stock`,
  `low` (with the quantity, only below `AUTOXIDE_INVENTORY_PUBLIC_LOW_THRESHOLD`), `out` or `preorder` per variant and
  store, never a large number; `ETag` and a 5-second cache. `Inventory::in_stock_at` uses the same rule (warehouse
  sources, bundles).
