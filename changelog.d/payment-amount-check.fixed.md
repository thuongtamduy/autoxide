- Modules `plans` and `bookings`: a `ledger.payment_succeeded` event whose amount is less than what is
  due (a short payment accepted during reconciliation) no longer marks a billing period paid or
  confirms a booking; a short booking deposit is refunded instead.
