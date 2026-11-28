# Autoxide TypeScript SDK

A typed, dependency-free client for the Autoxide HTTP API. The types in `src/schema.ts` are generated
from `docs/openapi.json` by `cargo xtask sdk`, so they always match the server of the same version.

Vietnamese: [`README.vi.md`](README.vi.md).

## Getting it

The SDK is not on the npm registry. Either:

- copy the `sdk/typescript/src` folder into your project, or
- install the package attached to each release, e.g.
  `npm install https://github.com/<owner>/autoxide/releases/download/vX.Y.Z/autoxide-sdk-X.Y.Z.tgz`.

It is TypeScript source (`.ts`), for projects that compile TypeScript (Vite, Next.js, `tsc`, Bun, Deno).
Use the SDK of the same version as your server.

## Use

```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk_…", language: "en" });

// Path and query parameters, bodies and results are typed from the API contract.
const menu = await api.get("/api/v1/menus/public/{key}", { path: { key: "main" }, query: { locale: "en" } });

// Writes can be retried safely with an idempotency key.
await api.post("/api/v1/menus", {
  body: { key: "footer", items: [{ label: "Contact", url: "mailto:hello@example.com" }] },
  idempotencyKey: crypto.randomUUID(),
});

// Every item of a list, page after page.
for await (const item of api.paginate("/api/v1/menus")) console.log(item.key);

try {
  await api.delete("/api/v1/menus/{id}", { path: { id: "…" } });
} catch (error) {
  if (error instanceof AutoxideError && error.code === "menus.menu_not_found") {
    // Branch on `code`; show `detail`; `errors` lists invalid fields; `retryAfter` for 429 / 503.
  }
}
```

- A `{"data": …}` answer gives the `data`; a page (`data` + `pagination`) stays whole.
- A `GET` with `ifNoneMatch: etag` answers `{ notModified: true }` when nothing changed.
- `token` may be a function, to read a fresh access token for each request.

### Headers and `ETag`: `requestWithHeaders` and `ifMatch`

`requestWithHeaders` gives what `request` gives, plus the status and the headers:

```ts
const { data: entry, etag, status, headers } = await api.requestWithHeaders("get", "/api/v1/content/entries/{id}", {
  path: { id },
});

// Change it only if nobody changed it since: `If-Match`. Otherwise 412 `precondition_failed`.
await api.patch("/api/v1/content/entries/{id}", { path: { id }, ifMatch: etag, body: { title: "New title" } });
```

`data` can be `{ notModified: true }` only when you pass `ifNoneMatch`.

### Refreshing the access token by itself: `refresh`

```ts
let tokens = { access: "", refresh: "" };
const auth = createClient({ baseUrl: "https://api.example.com" }); // login, refresh, logout
const api = createClient({
  baseUrl: "https://api.example.com",
  token: () => tokens.access,
  refresh: async () => {
    const next = await auth.post("/api/v1/auth/refresh", { body: { refresh_token: tokens.refresh } });
    tokens = { access: next.access_token, refresh: next.refresh_token };
  },
});
```

When a request gets `401 auth.token_expired`, the client calls `refresh` and sends the request once more with
the new token. Requests that fail at the same time wait for **one** call: a refresh token works once, and two
refreshes with it end the session (`auth.refresh_token_reused`). If `refresh` throws, the request rejects with
that error (sign the user in again). Other `401` codes are not refreshed.

Building a web app (CORS, storing tokens, several tabs, errors, uploads, a whole `api.ts`):
[`docs/guides/web-app.md`](../../docs/guides/web-app.md). Task-by-task guides with `curl` and SDK examples:
[`docs/guides/`](../../docs/guides/).

## For contributors

`src/schema.ts` is generated: change the API, regenerate `docs/openapi.json`, then run `cargo xtask sdk`.
The gate runs `cargo xtask sdk --check`; CI type-checks the SDK and the guide examples with
`npm ci && npx tsc --noEmit` (TypeScript 7, the development version) and again with TypeScript 5.0.4, and runs the
client's tests with `npm test` (`node --test`, Node 22.18 or newer, no extra package).

Supported TypeScript for projects that import the SDK: **5.0 or newer** (checked at 5.0.4, 5.9.3, 6.0.3 and 7.0.2). `moduleResolution: "Bundler"` needs 5.0.
