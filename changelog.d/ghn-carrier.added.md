- New experimental module `ghn`: connector for the carrier Giao Hang Nhanh (GHN) behind `shipping` — connection test,
  quotes, bookings, labels, cancellation, tracking and status webhooks, with each tenant's own GHN account (token and
  shop id on the carrier account). Written from GHN's public documentation; not yet verified against the live
  service. Turn it on with `AUTOXIDE_MODULES=official,ghn`. Guide: `docs/guides/providers/ghn.md`.
- `shipping`: the `webhook_path` of a new carrier account now names the tenant
  (`/api/v1/shipping/webhooks/{carrier_code}/{hook}/{token}`), so a carrier needs no tenant header or tenant domain.
  Addresses given out before keep working.
