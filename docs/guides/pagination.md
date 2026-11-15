# Reading long lists

Vietnamese: [`pagination.vi.md`](pagination.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

Every list answers one page: `{"data": [...], "pagination": {"limit", "next_cursor"}}`. Ask for the next page with `cursor` set to `next_cursor`; `null` means there is no more. `limit` is 1 to 200 (50 by default); `order` is `desc` (newest first, the default) or `asc`. The cursor stays valid while you read, even when records are added.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

<!-- example: first-post -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" --json '{"kind":"post","title":"One"}'
# expect: 201
```

<!-- example: second-post -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" --json '{"kind":"post","title":"Two"}'
# expect: 201
```

<!-- example: third-post -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" --json '{"kind":"post","title":"Three"}'
# expect: 201
```

<!-- example: first-page -->
```bash
curl -sS "$API/api/v1/content/entries?limit=2&order=asc" -H "Authorization: Bearer $TOKEN"
# save: CURSOR=.pagination.next_cursor
```

<!-- example: next-page -->
```bash
curl -sS "$API/api/v1/content/entries?limit=2&order=asc&cursor=$CURSOR" -H "Authorization: Bearer $TOKEN"
```

Keep the other parameters (`limit`, `order`, filters) the same from page to page; a cursor from another list answers `422`.

## With the SDK

`paginate()` follows the cursors for you:

```ts
import { createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk_…" /* read it from your secret store */ });

let count = 0;
for await (const entry of api.paginate("/api/v1/content/entries", { query: { limit: 100, order: "asc" } })) {
  count += 1;
  console.log(entry.id, entry.title);
}
console.log(`${count} entries`);
```
