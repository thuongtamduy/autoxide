- Module `orders`: a guest checkout, guest cancel, guest address change or `payment-link` sent again
  with the same `Idempotency-Key` now answers with `Idempotent-Replayed: true`, as documented.
- Module `orders`: `payment-link` after the collection failed or expired at the partner starts a new
  attempt (new `payment_id`, `status_token` and `ledger_payment_id`; the old payment becomes
  `superseded`) instead of returning the dead link. If the partner still collects the old attempt,
  the money is refunded through the partner once, and `orders.payment_recorded` notes it.
