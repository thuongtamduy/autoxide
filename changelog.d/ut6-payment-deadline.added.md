- Module `ledger`: `NewPayment::expires_by` / `NewTransferPayment::expires_by` cap a payment's deadline (the
  link at the partner expires at the same moment); `orders` uses it so a new attempt never outlives the
  order's deadline.
