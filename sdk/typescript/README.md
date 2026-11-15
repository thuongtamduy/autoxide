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

Task-by-task guides with `curl` and SDK examples: [`docs/guides/`](../../docs/guides/).

## For contributors

`src/schema.ts` is generated: change the API, regenerate `docs/openapi.json`, then run `cargo xtask sdk`.
The gate runs `cargo xtask sdk --check`; CI type-checks the SDK and the guide examples with
`npm ci && npx tsc --noEmit`.
