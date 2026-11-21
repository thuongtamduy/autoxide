- Module `orders`: two checkouts of the same cart with different `Idempotency-Key`s no longer
  create two orders; the later one gets `orders.cart_closed` (409) and its holds are released.
- Module `orders`: closing a POS shift waits for sales being written and counts them in the expected
  cash; a sale can no longer be recorded in a shift that was just closed (`orders.shift_not_open`).
