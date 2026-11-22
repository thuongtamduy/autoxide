- Module `orders`: a buyer can start the payment of an order again without the shop's staff, when it
  could not be started at checkout or its link expired — a guest with the `Order-Token` header
  (`POST /api/v1/orders/public/orders/{id}/payment-link`) or a signed-in buyer
  (`POST /api/v1/orders/me/orders/{id}/payment-link`). Only for an order still waiting for payment
  (or whose balance was asked), with an optional new `return_url`, safe to retry with the same
  `Idempotency-Key`, and at most 10 times an hour per order (`orders.payment_retry_too_many`).
- Module `orders`: when the payment partner refuses to start a payment at checkout, the payment action
  (`kind = none`) now carries `error` with a stable `code` and a `message` saying what to fix, in the
  caller's language. The checkout still answers `201` as before.
