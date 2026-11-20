- Module `customers` (step 47): sales profiles of buyers keyed by account or phone number (keyed hash
  per shop), registration at the counter, purchase history and totals built from order events,
  masked phone and email for staff with an audited `GET /customers/{id}/contact`, manual and
  rule-based customer groups (total spent, orders, recency, tier, tags, units) plugged into carts of
  `orders` for price lists and promotions, MCP tools. Guide: `docs/CUSTOMERS.md`.
