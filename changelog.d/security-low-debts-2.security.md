- Module `auth`: `POST /api/v1/auth/me/password` takes `revoke_other_credentials`; `true` also removes every
  API key, passkey and external sign-in link of the account, as a password recovery does (L-7 of the 2026-11
  security review). Without it they keep working, as before.
- Module `ledger`: a payment's `return_url` must be on one of the tenant's own addresses: an `https://` address
  on a verified domain, or an origin listed in the new tenant setting `ledger.payments.return_origins`. Any
  other address is refused with the field error `ledger.return_url_not_allowed`, so a payment link cannot send
  the buyer to an outside site after paying (P-5). **Action needed:** a shop that sends `return_url` lists its
  website's origin in `ledger.payments.return_origins` (or verifies its domain) before upgrading.
