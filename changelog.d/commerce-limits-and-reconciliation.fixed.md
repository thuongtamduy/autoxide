- Module `shipping`: weights, dimensions and quantities sent for a quote, a shipment or its parcels have a ceiling
  (10,000,000 g and 10,000,000 mm per item, as `catalog`; 1,000 tonnes per shipment, `api::MAX_PARCEL_WEIGHT_GRAMS`).
  Values over it answer 422 (field errors, or `shipping.load_too_large`) instead of fees and weights silently capped
  at the largest number; a rate tier whose fee overflows answers `money.overflow`. The GHN connector refuses parcels
  over the same ceiling with `ghn.parcel_too_large` instead of sending a saturated weight.
- Module `orders`: a payment the ledger reports in another currency than its order is not counted; the event keeps
  the code `orders.payment_received_currency_mismatch` (then goes to the dead-letter queue), the error is logged with
  the order and payment, and `orders_payment_currency_mismatch_total` counts it. `docs/MONEY.md` says how to refund it.
  Counter payments now always carry their currency inside the service, so a new caller cannot skip the
  `orders.payment_currency_mismatch` check.
- Module `inventory`: order events refused by stock are counted in `inventory_order_event_unapplied_total{event}`;
  goods delivered after staff released their reservation log a warning and count
  `inventory_delivery_after_manual_release_total`. New alerts `AutoxideStockNotIssued` and
  `AutoxidePaymentCurrencyMismatch` in `deploy/prometheus/autoxide-alerts.yml`; `docs/INVENTORY.md` has a
  reconciliation guide, including orders delivered before 0.20.0.
