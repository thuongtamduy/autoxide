- `POST /api/v1/compliance/purposes`: the per-tenant limit `AUTOXIDE_COMPLIANCE_PURPOSES_TENANT_MAX` is exact
  under concurrent requests (the count and the insert run in one write transaction, serialised per tenant); before,
  several purposes created at the same moment could go past it.
- Lists check the caller's permission before their paging parameters: a caller without the permission now gets
  `403 permission_denied` instead of `422` for a bad `limit` / `cursor` / `order` on `GET /api/v1/auth/all-api-keys`,
  `/auth/guardrails`, `/auth/users/{id}/passkeys`, `/bookings/bookings`, `/bookings/services`, `/bookings/resources`,
  `/catalog/collections`, `/compliance/kyc/profiles`, `/content/entries/{id}/translation-requests`, `/files`,
  `/invoices/series`, `/invoices/mandates`, `/invoices/documents`, `/ledger/books`, `/ledger/accounts`,
  `/ledger/accounts/{id}/lines`, `/ledger/entries`, `/ledger/periods`, `/ledger/holds`, `/ledger/payouts`,
  `/ledger/bank-accounts`, `/ledger/exchange-rates`, `/ledger/payments`, `/ledger/exceptions`,
  `/ledger/reconciliations`, `/menus`, `/orders` and `/orders/carts`.
- `GET /api/v1/kernel/records/{kind}/{id}/usages`: the description now says that records of a module without a paged
  usage lookup of its own are ordered by a stable key, not newest first.
