# Analytics

*Tiếng Việt: [ANALYTICS.vi.md](ANALYTICS.vi.md)*

The `analytics` module counts the visits of your websites inside Autoxide: how many people came,
which pages and objects (products, posts) they viewed, where they came from (search, social
networks, UTM campaigns) and where a funnel such as "product → cart → paid order" loses them.
Nothing is sent to a third party, no cookie is set, and only daily totals are kept for long.

## How it works

1. Pages of the website send events to `POST /api/v1/analytics/public/events`.
2. Other modules declare their event types (e.g. `catalog.product_viewed`) and funnels; events
   the server knows better (a paid order) are taken from that module's own events, never from the
   browser.
3. Every hour a job adds the events up into daily totals per website. Reports only read those
   totals, so they stay fast on large sites. Today's numbers lag by up to an hour.
4. Single events are deleted after `AUTOXIDE_ANALYTICS_RAW_RETENTION_DAYS` (7); daily totals after
   `AUTOXIDE_ANALYTICS_RETENTION_DAYS` (790, two years and two months).

### Privacy by default

- **No cookie, no fingerprint.** A visit is told apart by a session key: a keyed hash of the
  website, the network block of the address (IPv4 /24, IPv6 /64) and a coarse browser label
  (`chrome|android|mobile`, no version). The key is random per day and deleted once the day is
  over: sessions of two days cannot be linked, and nobody can work an address back out.
- **Neither the address nor the full user agent is stored.** Query strings and fragments are cut
  from page paths (they may hold emails); only the host of the referring page is kept.
- **Per-person tracking only with consent.** When a signed-in user consented to the purpose
  `analytics.personal` in the consent ledger of the `compliance` module, their events carry their
  account, so modules can show "recently viewed". Withdrawing stops it at once and removes the
  link from past events; daily totals, which hold nobody, stay.
- Visitors over several days are the sum of the daily unique visitors: without cookies one person
  on two days counts twice.

## Sending events from a website

```js
const queue = [];
function track(type, extra = {}) {
  const params = new URLSearchParams(location.search);
  queue.push({
    type,
    path: location.pathname,
    referrer: document.referrer || undefined,
    utm: {
      source: params.get("utm_source") || undefined,
      medium: params.get("utm_medium") || undefined,
      campaign: params.get("utm_campaign") || undefined,
    },
    ...extra,
  });
}
function flush() {
  if (!queue.length) return;
  const events = queue.splice(0, 20);
  fetch("https://api.shop.example/api/v1/analytics/public/events", {
    method: "POST",
    keepalive: true, // still sent while the page closes, like sendBeacon
    credentials: "include", // only if signed-in users should be linked (with consent)
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ events }),
  });
}
track("analytics.page_view");
// On a product page:
track("catalog.product_viewed", { object: "catalog.product:0190…" });
addEventListener("visibilitychange", () => document.visibilityState === "hidden" && flush());
```

- At most 20 events per call. The answer is always `202` with `accepted` and `dropped` counts.
- `GET /api/v1/analytics/event-types` lists every type the installed modules declare, with their
  props and object kind.
- Events are dropped, never refused, when: the type is unknown or recorded by the server only;
  `props` holds an undeclared name or a wrong type; `object` is missing or of the wrong kind for a
  type that names one; the caller is a known bot or sends no user agent; the page (its `Origin`,
  else `Referer`) is not on this website; more than `AUTOXIDE_ANALYTICS_EVENTS_PER_MINUTE` (120)
  events came from the same network block in a minute.
- The website is the one of the domain called. A page served from another host than the API (one
  tenant, no verified domain) is accepted when its host is listed in
  `AUTOXIDE_ANALYTICS_ALLOWED_HOSTS`. Set `?site=<unit id>` when the API is not called on the
  website's own domain.

## Reports

All need `analytics.read` at the website's organization unit; `site` defaults to the website of
the whole tenant, `from` / `to` (UTC days) to the last 30 days, at most 400 days at once.

| Endpoint | What |
|---|---|
| `GET /api/v1/analytics/overview` | visitors, page views, events, bounce rate, top sources, one entry per day |
| `GET /api/v1/analytics/pages` | most viewed pages |
| `GET /api/v1/analytics/sources` | where visits came from: `utm_source`, else the referring site, else `direct` |
| `GET /api/v1/analytics/campaigns` | `utm_campaign` values |
| `GET /api/v1/analytics/objects?kind=catalog.product` | most viewed objects |
| `GET /api/v1/analytics/types` | events by type |
| `GET /api/v1/analytics/funnels`, `/funnels/{key}` | declared funnels; sessions per step |

The **bounce rate** is the share of visits that saw exactly one page. **Funnel** steps are counted
on their own (cookieless visits cannot be linked to an order recorded by the server), so a later
step may exceed an earlier one.

MCP tools (read only): `analytics.overview`, `analytics.top_pages`, `analytics.top_objects`,
`analytics.funnel`. The summary metric `analytics.visitors` (last 7 days) appears on the overview
page of the admin interface.

## For module authors

```rust
use autoxide_kernel::analytics::{AnalyticsEvent, Funnel, Prop, PropKind, ServerHit};

const VARIANT: &[Prop] = &[Prop::new("variant", PropKind::Text)];

fn analytics_events(&self) -> Vec<AnalyticsEvent> {
    vec![
        AnalyticsEvent::browser("catalog.product_viewed", "A product page was shown.")
            .object_kind("catalog.product")
            .props(VARIANT),
        // Taken from this module's own event `orders.order_confirmed`.
        AnalyticsEvent::server("orders.purchased", "An order was paid.", "orders.order_confirmed", |event| {
            Ok(Some(ServerHit::new(event.occurred_at).object(format!("orders.order:{}", event.id))))
        }),
    ]
}

fn funnels(&self) -> Vec<Funnel> {
    vec![Funnel::new("orders.checkout", "From product to paid order.",
        &["catalog.product_viewed", "orders.cart_added", "orders.purchased"])]
}
```

Other modules read numbers through `autoxide_analytics::api::Analytics`: `top_objects` (what is
trending) and `recently_viewed` (empty without consent).

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `AUTOXIDE_ANALYTICS_ENABLED` | `true` | take events at all |
| `AUTOXIDE_ANALYTICS_RAW_RETENTION_DAYS` | `7` | days single events are kept (2–90) |
| `AUTOXIDE_ANALYTICS_RETENTION_DAYS` | `790` | days daily totals are kept (30–3650) |
| `AUTOXIDE_ANALYTICS_EVENTS_PER_MINUTE` | `120` | events per network block per minute |
| `AUTOXIDE_ANALYTICS_ALLOWED_HOSTS` | empty | extra website hosts allowed to send |

## Legal notes

Vietnamese law (Personal Data Protection Law 91/2025/QH15, Decree 356/2025) asks for consent per
purpose before linking behaviour to a person, and for keeping only what is needed. Whether
anonymous daily totals are personal data, and whether a cookieless site needs a consent banner,
is not settled: check the original texts or ask someone qualified for your case. Behaviour-based
advertising messages need consent (Decree 91/2020/NĐ-CP).
