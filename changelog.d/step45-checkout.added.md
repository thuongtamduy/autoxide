- Module `orders` (step 45): checkout (`POST /checkout`, `Idempotency-Key` required, same key → same order even
  days later) holds stock and promotion uses per order, numbers orders without gaps (`<year>-000001`), takes payment
  through the ledger (payment partner or VietQR to the seller's account, `payment_actions[]`) or on delivery,
  confirms on `ledger.payment_succeeded`, cancels unpaid orders at the ledger's payment deadline and refunds late
  payments; public payment status for the QR screen (`GET /public/payments/{token}`); `GET /me/orders`,
  `GET /` and `GET /{id}` for staff.
