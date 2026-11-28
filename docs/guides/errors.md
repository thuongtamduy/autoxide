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

## Changing a record: `If-Match`, `428` and `412`

Reading one record returns its version in the `ETag` header. Endpoints that change a record take it back in `If-Match`, so two people never overwrite each other. Endpoints whose description says `If-Match` is required answer `428` (code `precondition_required`) without it; a version that is no longer current gets `412` (code `precondition_failed`): read the record again, apply your change to what you read, and send the new `ETag`.

<!-- example: create-entry -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" \
  --json '{"kind":"post","title":"First title"}'
# expect: 201
# save: ENTRY=.data.id
```

<!-- example: read-entry -->
```bash
curl -sS "$API/api/v1/content/entries/$ENTRY" -H "Authorization: Bearer $TOKEN"
# save: ENTRY_ETAG=header:etag
```

<!-- example: change-without-version -->
```bash
curl -sS -X PATCH "$API/api/v1/content/entries/$ENTRY" -H "Authorization: Bearer $TOKEN" \
  --json '{"title":"Second title"}'
# expect: 428
```

<!-- example: change-with-version -->
```bash
curl -sS -X PATCH "$API/api/v1/content/entries/$ENTRY" -H "Authorization: Bearer $TOKEN" \
  -H "If-Match: $ENTRY_ETAG" --json '{"title":"Second title"}'
# expect: 200
```

<!-- example: change-with-old-version -->
```bash
curl -sS -X PATCH "$API/api/v1/content/entries/$ENTRY" -H "Authorization: Bearer $TOKEN" \
  -H "If-Match: $ENTRY_ETAG" --json '{"title":"Third title"}'
# expect: 412
```

## What your app should do

The codes a web or mobile app meets most, and what to do with each. Branch on `code`; show `detail` to the
user (it is already in their language). Codes of a module start with its name (`content.not_found`,
`auth.email_taken`): treat them by their status when you have no special case.

| Status | `code` | What it means | What the app does |
| ------ | ------ | ------------- | ----------------- |
| 401 | `auth.token_expired` | The access token expired | Refresh once (`POST /api/v1/auth/refresh`) and send the request again; the SDK does it with the `refresh` option |
| 401 | `auth.missing_token`, `auth.invalid_token`, `auth.session_ended` | No token, a broken one, or the session was signed out (logout, password change, revoked) | Forget the tokens, show the sign-in page |
| 401 | `auth.invalid_refresh_token`, `auth.refresh_token_reused` | From `/auth/refresh`: the refresh token is unknown, expired, or was already used (the session is now ended) | Sign in again. Never refresh twice at the same time |
| 401 | `auth.invalid_credentials` | From `/auth/login`: wrong email or password | Say so, without telling which one |
| 401 | `auth.invalid_mfa_code`, `auth.invalid_mfa_token` | From `/auth/login/mfa`: wrong or already used code; the sign-in attempt expired or ran out of tries | Ask for the next code; for `auth.invalid_mfa_token` start again with the password |
| 401 | `step_up_required` | A sensitive action needs a fresh proof that it is you | Ask for the password or code, call `POST /api/v1/auth/me/reauth`, retry |
| 403 | `permission_denied` | The user lacks the permission (the `detail` names it) | Hide the action; offer to ask an administrator |
| 403 | `auth.user_disabled` | The account is disabled | Tell the user; do not retry |
| 404 | `not_found`, `<module>.…_not_found` | Gone, or never existed (or in another tenant) | Refresh the list or go back |
| 409 | `idempotency_key_in_progress` | The first request with this key is still running | Wait `Retry-After` (1 s) and send the same request with the **same** key |
| 409 | `<module>.…_taken`, `conflict` | A unique value is taken, or the state does not allow it | Show `detail` next to the field |
| 412 | `precondition_failed` | Someone changed the record since you read it | Read it again, show the new version, apply the change again |
| 413 | `payload_too_large` | The body or file is larger than the server accepts | Tell the user the limit; upload big files [straight to storage](direct-uploads.md) |
| 422 | `validation_failed` | Invalid input; **every** invalid field is in `errors[]` | Show each `errors[].detail` next to its `errors[].field` |
| 422 | `invalid_body` | Unknown or misspelt field, wrong JSON type | A bug in the app: fix the request |
| 422 | `idempotency_key_reused` | The same key was sent with a different request | A bug in the app: one new key per user action |
| 428 | `precondition_required` | The endpoint needs `If-Match` | Read the record, send its `ETag` in `If-Match` |
| 429 | `rate_limited` | Too many requests from this client address (or with this API key) | Wait `Retry-After` seconds, then send fewer requests |
| 429 | `tenant_rate_limited` | The whole tenant (every user and key together) is over its limit | Wait `Retry-After` seconds |
| 429 | `auth.login_throttled` | Too many failed sign-ins for this email (from this address, or from everywhere) | **Do not retry by yourself.** Tell the user to wait `Retry-After` seconds (often several minutes); signing in with the right password is refused too until then |
| 503 | `request_timeout`, `shutting_down` | The request took too long, or the server is restarting | Wait `Retry-After` (or a few seconds) and retry; send writes with an `Idempotency-Key` |

The two common `429`s are counted apart: `rate_limited` counts **requests** per client address (every
endpoint, `AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE`), while `auth.login_throttled` counts **failed sign-ins**
per email (`AUTOXIDE_AUTH_LOGIN_MAX_FAILURES` within `AUTOXIDE_AUTH_LOGIN_LOCK_MINUTES`). A sign-in form
must never retry by itself.

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
