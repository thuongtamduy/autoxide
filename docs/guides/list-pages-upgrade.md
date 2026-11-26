# Upgrading clients: every list is a page

Vietnamese: [`list-pages-upgrade.vi.md`](list-pages-upgrade.vi.md). For reading lists in general, see
[Reading long lists](pagination.md).

Before version 1.0, every `GET` endpoint that returns a list was changed in place to return **one page**. The path did
not change; the shape of the answer did. A client written for the old shape reads only the first 50 items and must be
updated.

## What changed

Before, some lists answered the whole list in `data`:

```json
{"data": [{"id": "…"}, {"id": "…"}]}
```

Now every list answers one page, with `pagination` next to `data`:

```json
{"data": [{"id": "…"}, {"id": "…"}], "pagination": {"limit": 50, "next_cursor": "…"}}
```

- `data` holds the items of this page, as before.
- `pagination.limit` is the page size that was applied.
- `pagination.next_cursor` is where the next page starts; `null` (or absent) means this is the last page.
- `pagination.total` is the number of items matching your filters, over all pages. It is only there on the first page
  of a request sent with `count=true`.

The same holds for the MCP tools that list things: they take the same parameters and answer the same
`{"data": […], "pagination": {…}}`.

## How to read a whole list

Every list takes the same four query parameters:

| Parameter | Meaning |
| --------- | ------- |
| `limit`   | Items per page, 1 to 200. Defaults to 50. |
| `cursor`  | The `next_cursor` of the previous page. Leave it out for the first page. |
| `order`   | `asc` or `desc`. The default depends on the list (see below). |
| `count`   | `true` to also get `pagination.total` on the first page. Counting costs a query: ask only when you show it. |

Ask for the first page, then, while `next_cursor` is not `null`, ask again with `cursor` set to it **and the same
filters, `order` and `sort`**. A cursor from another list, or from the same list with another `sort`, answers `422`
with the problem on `cursor`. The cursor stays valid while you read, even when items are added.

```bash
curl -sS "$API/api/v1/content/entries?limit=100&count=true" -H "Authorization: Bearer $TOKEN"
```

```bash
curl -sS "$API/api/v1/content/entries?limit=100&cursor=$CURSOR" -H "Authorization: Bearer $TOKEN"
```

Stop when `next_cursor` is `null`, **not** when a page is shorter than `limit`. Lists that check your permission on
each item (for example `GET /api/v1/kernel/custom-fields` without `target`, or the approval requests) read a bounded
number of rows per request: a page may hold fewer items than `limit`, even none, and still have a `next_cursor`.
Their description in the API reference says so.

## Which order a list uses

Each list says in its description what it is ordered by. There are three kinds:

- **Newest first** — lists with no order other than when items were created (orders, refunds, events, notes…):
  `order` defaults to `desc`; `order=asc` gives the oldest first.
- **Natural order** — lists with a meaningful order of their own (position, name, code, depth in a tree…): `order`
  defaults to `asc` (first position, A to Z); `order=desc` reverses it.
- **Follows `sort`** — lists with a `sort` parameter: the default `order` follows the chosen column — `desc` (newest
  first) for times such as `created_at` and `updated_at`, `asc` for names, codes, positions and sizes. The description
  of `sort` lists the default of each value. Sending `order` yourself always wins.

The order is stable from page to page: items are never repeated or skipped while you follow `next_cursor`.

## With the TypeScript SDK

`paginate()` follows `next_cursor` for you and yields the items one by one:

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk1_…" /* read it from your secret store */ });

// Every record that uses an image, whatever the number of pages.
for await (const usage of api.paginate("/api/v1/kernel/records/{kind}/{id}/usages", {
  path: { kind: "media.item", id: "0190a4c2-0000-7000-8000-000000000001" },
  query: { limit: 100 },
})) {
  console.log(usage.kind, usage.id, usage.title);
}
```

To show one page at a time (a table with a "next" button) and a total, call `get()`; a list keeps its `pagination`:

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk1_…" /* read it from your secret store */ });

const first = await api.get("/api/v1/content/entries", { query: { limit: 20, count: true } });
console.log(`${first.pagination.total ?? "?"} entries`);
let cursor = first.pagination.next_cursor;
while (cursor) {
  const page = await api.get("/api/v1/content/entries", { query: { limit: 20, cursor } });
  for (const entry of page.data) console.log(entry.id, entry.title);
  cursor = page.pagination.next_cursor;
}
```

## With MCP tools

A tool that lists things takes `limit`, `cursor`, `order` and `count` (plus `sort` when the REST endpoint has it) with
the same meaning, and answers `{"data": […], "pagination": {…}}`. An agent calls the tool again with `cursor` set to
`pagination.next_cursor` and the same other arguments until it is `null`. See [Connecting an AI agent over
MCP](mcp.md).

## Exceptions

Three lookups answer a list in `data` without `pagination`, because they return at most one item per key you send
(at most 100 keys), in the order you sent them: `GET /api/v1/catalog/variants?code=…`,
`GET /api/v1/comments/public/counts` and `GET /api/v1/inventory/public/availability`.

## What changed, endpoint by endpoint

The changelog lists every endpoint and tool that changed, with its default order and the behaviour changes (lists
whose order changed, `limit` parameters of a single endpoint replaced by the shared one): see the "Changed" section
of `CHANGELOG.md` for this release, entries marked **Breaking (pre-1.0, D-20261125-list-pages)**.
