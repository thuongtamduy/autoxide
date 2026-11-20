- Module `orders`: warranties opened when an order is delivered — per serial number recorded while
  preparing the order, or per order line — looked up by serial or buyer phone, listed for the buyer,
  and warranty claims with a claim number, a due date and a handling status (MCP tools
  `lookup_warranty`, `update_warranty_claim`).
- Module `orders`: installment payments through a licensed partner chosen at checkout
  (`payment_method: installment` with `{partner, months}`).
- Module `orders`: shop-defined fields buyers fill in at checkout (`custom_fields`, custom field
  target `orders.order`), shown on the order and available to confirmation messages.
- Module `orders`: orders show `buyer_cancel_until`, `buyer_can_cancel` and `return_until`; carts
  list the usable `payment_methods`; review statistics count reviews per variant.
