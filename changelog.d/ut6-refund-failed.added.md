- Module `ledger`: a refund the partner refuses opens a `refund_failed` item in the reconciliation queue
  (unless the payment's `late_payment` item is still open); it closes on its own once the payment is
  refunded in full. `autoxide_ledger::api::refund_refused` tells a final refusal from an error worth
  sending again (409, 429, 5xx).
