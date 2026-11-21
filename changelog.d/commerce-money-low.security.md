- Module `customers`: a gift card attached to an account can only be spent or attached by that
  account; a referral reward is taken back when the qualifying order is refunded below the
  minimum order amount.
- Module `ledger`: two bank transfers for the same VietQR payment matched at the same time never
  lose the extra transfer; it always opens a `duplicate_payment` item.
- Module `orders`: a cash-on-delivery collection for a shipment with no matching delivery is
  counted once, even when the event arrives again.
- Module `promotions`: `%` and `_` in the `q` search of promotions match themselves.
