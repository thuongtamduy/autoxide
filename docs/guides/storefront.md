# Building a shop's home page from the API

Vietnamese: [`storefront.vi.md`](storefront.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

The website draws its own pages; Autoxide gives it the data: **banners** in named spots of the page, **product collections**, and the shop's **brand and contact details**. Changing a banner or a collection is a data change (REST, or an AI agent over MCP), never a change of the website's code. Full reference: [`docs/CONTENT.md`](../CONTENT.md) (banners), [`docs/CATALOG.md`](../CATALOG.md) (collections).

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

## Placements: where banners go

A **placement** is a named spot (`top_strip`, `home_hero`, `sidebar_left`, `popup`…) that says how its banners are shown (`single`, `carousel`, `grid`, `stack`, `strip`, `popup`), which picture ratio and size it takes, on which devices and kinds of pages. A new tenant gets a standard set; a tenant made before this feature adds the missing ones:

<!-- example: presets -->
```bash
curl -sS -X POST "$API/api/v1/content/placements/presets" -H "Authorization: Bearer $TOKEN"
```

## A banner

A text banner in the top strip, linking to a page of the website. `tone` is a role of the theme (`primary`, `secondary`, `accent`, `neutral`), never a colour code, so a new theme recolours it. Image banners take a **public** picture (`media_id`) with the placement's ratio; a link can also point to an object, e.g. `{"kind": "catalog.collection", "id": "…"}`.

<!-- example: create-banner -->
```bash
curl -sS "$API/api/v1/content/banners" -H "Authorization: Bearer $TOKEN" \
  --json '{"placement":"top_strip","locale":"en","kind":"text","title":"Free delivery over 500,000 VND","tone":"accent","link":{"url":"/shipping"},"status":"active"}'
# expect: 201
```

An AI agent does the same with the MCP tools `content.save_banner`, `content.order_placement`, `catalog.save_collection`: they only write **drafts** marked `ai_assisted`, and a person turns them on.

## A collection

A collection groups products for a page: chosen by hand in order (`manual`), or by a rule (categories, attributes, price range, new, on promotion), sorted by `newest`, `price_asc`, `price_desc` or `best_selling`.

<!-- example: create-collection -->
```bash
curl -sS "$API/api/v1/catalog/collections" -H "Authorization: Bearer $TOKEN" \
  --json '{"kind":"rule","rules":{"new_within_days":30},"status":"active","texts":{"en":{"title":"New this month"}}}'
# expect: 201
```

## The home page, as the website reads it

Three public calls, no sign-in. Every placement of the page in one call — banners active now, in their order, with `href` ready and picture `variants` for `srcset`:

<!-- example: public-placements -->
```bash
curl -sS "$API/api/v1/content/public/placements?keys=top_strip,home_hero,home_middle,popup&locale=en&page_kind=home"
# save: PLACEMENTS_ETAG=header:etag
```

The shop's brand, contact details, address and opening hours:

<!-- example: public-storefront -->
```bash
curl -sS "$API/api/v1/organizations/public/storefront"
```

A collection page, with one page of its products (same cards as the product list):

<!-- example: public-collection -->
```bash
curl -sS "$API/api/v1/catalog/public/collections/new-this-month?locale=en"
```

A shop selling online in Vietnam also shows its owner and the Ministry of Industry and Trade badge in the footer: `GET /api/v1/compliance/public/owner` and `GET /api/v1/compliance/public/badge`.

## Cache

Public answers carry an `ETag` (send it in `If-None-Match` for `304`) and a `Cache-Control` that ends when the next banner starts or ends, so a CDN never keeps an expired banner:

<!-- example: placements-not-modified -->
```bash
curl -sS "$API/api/v1/content/public/placements?keys=top_strip,home_hero,home_middle,popup&locale=en&page_kind=home" -H "If-None-Match: $PLACEMENTS_ETAG"
# expect: 304
```

## Rules for the website

- Draw banners in the order given; never shuffle them.
- A `popup` has a visible close button, respects its `frequency`, and never shows on cart, checkout, sign-in or account pages.
- A `carousel` does not autoplay when the viewer asks for reduced motion (`prefers-reduced-motion`) and can be paused.
- Banners and collections with `ai_assisted: true` carry an "AI-assisted" label (Vietnam's Law on Artificial Intelligence — check the original text).

## With the SDK

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com" });

const [placements, shop, collection] = await Promise.all([
  api.get("/api/v1/content/public/placements", {
    query: { keys: "top_strip,home_hero,home_middle,popup", locale: "en", page_kind: "home" },
  }),
  api.get("/api/v1/organizations/public/storefront"),
  api.get("/api/v1/catalog/public/collections/{slug}", {
    path: { slug: "new-this-month" },
    query: { locale: "en" },
  }),
]);

for (const placement of placements.placements) {
  console.log(placement.key, placement.display, placement.banners.length);
}
console.log(shop.brand.name, collection.title, collection.ai_assisted);
```
