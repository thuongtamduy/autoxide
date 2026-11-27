- Module `orders` and `ledger`: one rule for late money. The order decides whether money that arrived after
  the deadline (or after the partner reported the payment failed) is kept or refunded; when the order
  goes on, the payment's `late_payment` item now closes on its own (resolution `kept`) instead of asking
  the operator to refund an order in delivery. New `Ledger::keep_late_payment` (default: does nothing).
- Module `ledger`: a refund no partner can make (no partner configured for the payment, such as a bank
  transfer, or the partner does not refund it) also opens a `refund_failed` item, once per `reference`,
  while money is left to refund. A mistake in the request itself (more than what is left, another
  currency) only answers the error.
- Module `orders`: `payment-link` racing the order's deadline answers `orders.payment_expired` (not
  `orders.payment_method_unavailable`); the refund endpoints document the 409 / 429 answers that keep a
  refund `pending` to send again.
