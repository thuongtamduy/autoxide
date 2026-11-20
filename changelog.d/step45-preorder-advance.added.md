- Module `orders`: partial advance payment for pre-orders (`orders.preorder_advance` setting) —
  carts show what is paid now and later, buyers paying online pay the items in stock plus the
  advance when ordering, are asked for the balance once the items are ready
  (`POST /me/orders/{id}/payments`), pre-ordered items ship only once paid, overdue balances
  cancel the pre-ordered items, and a buyer cancelling after the cut-off may have part of the
  advance kept as published. It is an advance payment, not a deposit.
