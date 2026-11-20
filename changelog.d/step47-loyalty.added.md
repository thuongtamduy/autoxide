- Module `customers`: loyalty points in lots with their own expiry (earned when an order completes,
  oldest lot used first, never below zero under concurrent use), membership tiers by 12-month
  spending with an earning multiplier, manual adjustments with a reference, birthday / purchase /
  points-exchange vouchers through promotions, and the unused points kept as a liability in the
  ledger at their value when earned.
- Module `orders`: checkout accepts `tenders` (e.g. `{"kind": "points", "input": {"points": 500}}`)
  to pay part or all of an order; the payment partner collects only the rest, and an order paid in
  full by tenders is confirmed at once.
