- Stock now leaves the warehouse when an order's goods reach the buyer: `inventory` writes the `sale` lines on
  the new event `orders.fulfillment_delivered` (one shipment) and on `orders.order_delivered` (now with
  `after.delivered_lines`), so `on_hand` and `reserved` drop, the reservation becomes `fulfilled` and the order's
  `fulfillment_status` becomes `fulfilled` (`partial` once some units of a split order were delivered). Before,
  nothing issued the stock: `on_hand` never went down after a sale (found by the release-candidate trial, UT-1).
  Each event line carries `delivered_quantity` (units of the line delivered so far) and stock is issued up to that
  total, so events arriving again, late or out of order, or after a manual `fulfil` when the parcel left, issue
  nothing more. Serial-tracked units issue with the serials recorded on the order line, each from the warehouse
  holding it; units still without serials stay reserved until they are recorded (new event
  `orders.line_serials_recorded`). Because the goods are gone, this path may take units of an expired lot and
  issues units whose reservation had expired or been released (both logged); a delivery that cannot be issued
  is logged with its error code and counted in `inventory_delivery_unissued_total`.
  **Upgrading:** orders delivered before this version keep their reservation `confirmed`; see "Reservations (for orders)" in
  `docs/INVENTORY.md` to release them and record the sale by hand.
