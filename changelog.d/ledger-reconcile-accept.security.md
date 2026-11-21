- Module `ledger`: resolving a reconciliation item with `match` now requires the amount collected
  to equal the amount due and the payment to be the one the item is about and still waiting for
  its money; a different amount needs the new action `accept_received`. The event
  `ledger.payment_succeeded` carries the amount actually collected.
