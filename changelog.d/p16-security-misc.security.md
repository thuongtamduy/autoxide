- Module `compliance`: the `return_url` of an identity verification session must be on one of the tenant's
  own addresses — `https://` on a verified domain, or an origin listed in the new tenant setting
  `compliance.kyc.return_origins`; any other address answers 422 (`compliance.return_url_not_allowed` on the
  `return_url` field). **Action needed:** list your site's origin before sending `return_url` (L-11).
- Module `automations`: a rule saved by a signed-in person or a credential never runs with a wider scope
  than at saving time — a later wider grant (another unit, the whole tenant) no longer reaches the rule.
- Events: `auth.access_revoked` is never dropped after repeated delivery failures; it is retried hourly
  like account deletion, so automation rules of a taken-over account always pause.
- Event WebSocket (`GET /api/v1/kernel/events/ws`): a browser page whose `Origin` is neither in
  `AUTOXIDE_HTTP_CORS_ORIGINS` nor a verified tenant domain gets 403 `kernel.ws_origin_not_allowed`.
- Requests with random `Host` values no longer evict real tenant domains from the server's cache: the website
  of a `Host` is looked up in the snapshot of verified domains.
