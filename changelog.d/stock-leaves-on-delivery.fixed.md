- Stock now leaves the warehouse when an order's goods reach the buyer: `inventory` writes the `sale` lines on
  the new event `orders.fulfillment_delivered` (one shipment) and on `orders.order_delivered` (the units no shipment
  carried, listed in the new `after.delivered_lines`), so `on_hand` and `reserved` drop, the reservation becomes
  `fulfilled` and the order's `fulfillment_status` becomes `fulfilled` (`partial` after a split shipment). Before,
  nothing issued the stock: `on_hand` never went down after a sale (found by the release-candidate trial, UT-1).
  Receiving an event again, late or out of order issues nothing more. Serial-tracked units issue with the serials
  recorded on the order line; units still without serials stay reserved until they are recorded (new event
  `orders.line_serials_recorded`). This path may issue units of an expired lot (logged), since the goods are gone.
