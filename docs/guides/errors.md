# Reading errors

Vietnamese: [`errors.vi.md`](errors.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

Every error is RFC 9457 Problem Details (`application/problem+json`):

- `code` — stable, for programs: **branch on it**, never on the text. A module's codes start with its name (`content.not_found`).
- `detail` — a sentence for people, in the language of `Accept-Language` (`en` by default, or `vi`).
- `errors` — on `422`, one entry per invalid input field: `field` (e.g. `items[0].url`), `code`, `detail`. Every problem of the request is listed at once.
- the `X-Request-Id` header — quote it when you report a problem.
- `Retry-After` (header) — on `429` (too many requests) and `503` (the server is restarting), the seconds to wait.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

Not signed in:

<!-- example: not-signed-in -->
```bash
curl -sS "$API/api/v1/content/entries"
# expect: 401
```

Invalid input, with the details in Vietnamese:

<!-- example: invalid-input -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" -H 'Accept-Language: vi' \
  --json '{"kind":"essay","title":"x"}'
# expect: 422
```

A misspelt or unknown field in the body is refused with `422` (code `invalid_body`, the `detail` names the field and the fields expected) rather than silently ignored:

<!-- example: unknown-field -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","pasword":"correct horse battery staple"}'
# expect: 422
```

A record that does not exist:

<!-- example: not-found -->
```bash
curl -sS "$API/api/v1/content/entries/0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11" -H "Authorization: Bearer $TOKEN"
# expect: 404
```

## With the SDK

Errors throw an `AutoxideError`:

```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk1_…" /* read it from your secret store */, language: "vi" });

try {
  await api.post("/api/v1/content/entries", { body: { kind: "post", title: "" } });
} catch (error) {
  if (!(error instanceof AutoxideError)) throw error;
  switch (error.code) {
    case "validation_failed":
      for (const field of error.errors) console.log(field.field, field.detail);
      break;
    case "rate_limited":
      console.log(`try again in ${error.retryAfter} s`);
      break;
    default:
      console.log(error.status, error.detail, error.requestId);
  }
}
```
