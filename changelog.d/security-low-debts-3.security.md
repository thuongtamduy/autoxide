- Tenant domains: a domain another tenant has only claimed (`pending`) no longer blocks you for 7 days. Several
  tenants may wait on the same domain; the first to verify it gets it, and the others' claims turn `failed`
  with the reason (P-4 of the 2026-11 security review).
- Module `automations`: a rule of an organization unit other than the tenant's root starts only for events of
  that unit or its branch (the event's `org_id`); events naming no unit reach only tenant-wide rules (L-8).
- Module `orders`: a `return_url` that is not an http(s) URL is reported as `orders.return_url_invalid`, apart
  from `orders.return_url_not_allowed` (an address that is not the shop's).
- HTTP server: an open WebSocket counts towards `AUTOXIDE_HTTP_MAX_CONNECTIONS` for as long as it stays open, and
  a connection that has sent nothing yet no longer delays a graceful shutdown (L-10 follow-up).
