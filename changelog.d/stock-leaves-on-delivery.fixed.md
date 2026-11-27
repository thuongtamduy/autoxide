- Stock now leaves the warehouse when an order's goods reach the buyer: `inventory` writes the `sale` lines on
  `orders.order_delivered` (the rest of the order) and on the new event `orders.fulfillment_delivered` (one
  shipment of a split order), so `on_hand` and `reserved` drop, the reservation becomes `fulfilled` and the
  order's `fulfillment_status` becomes `fulfilled` (`partial` after a split shipment). Before, nothing issued the
  stock: `on_hand` never went down after a sale (found by the release-candidate trial, UT-1). Receiving an event
  again issues nothing more.
