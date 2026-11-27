- Module `orders`: a guest checkout, guest cancel, guest address change or `payment-link` sent again
  with the same `Idempotency-Key` now answers with `Idempotent-Replayed: true`, as documented.
- Module `orders`: `payment-link` after the partner reported the collection failed starts a new
  attempt (new `payment_id`, `status_token` and `ledger_payment_id`; the old payment becomes
  `superseded`) instead of returning the dead link. If the partner still collects the old attempt,
  the money is refunded through the partner once, and `orders.payment_recorded` notes it.
- Module `ledger`: money collected for a payment the partner had reported failed opens a `late_payment`
  item in the reconciliation queue, as money arriving after the deadline already did.
- Module `ledger`: a `late_payment` item closes on its own (`resolved`, resolution `refunded`) once the
  payment is refunded in full through the partner, so the queue shows only payments to refund by hand.
