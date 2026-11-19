- Module `inventory`: listens to `orders.preorder_cancelled` and releases only the cancelled pre-order lines of an
  order that stays open — the reserved stock and the pre-order places come back once, even when the event is delivered
  again.
