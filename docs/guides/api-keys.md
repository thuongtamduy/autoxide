# API keys for programs and AI agents

Vietnamese: [`api-keys.vi.md`](api-keys.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

A program (a website build, a script, an AI agent) uses an **API key** instead of signing in. Give each key only the permissions it needs: it can never do more than you can, and a request beyond its permissions answers `403`.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

## Create a key

Creating a key asks for your current password, so a stolen access token alone cannot mint a key that outlives it. Instead of the password you may send `totp_code` (a code from your authenticator app, when two-step sign-in is on) or a passkey (`passkey_ceremony_id` from `POST /api/v1/auth/login/passkey/options` and `passkey_credential`, the browser's answer) — exactly one of the three. The key is shown **once**: store it now.

<!-- example: create-key -->
```bash
curl -sS "$API/api/v1/auth/api-keys" -H "Authorization: Bearer $TOKEN" \
  --json '{"name":"Website build","permissions":["content.read"],"current_password":"correct horse battery staple"}'
# expect: 201
# save: KEY=.data.key
# save: KEY_ID=.data.id
```

## Use it

Send it like a token: `Authorization: Bearer axk1_…`. A key looks like `axk1_<tenant ID, 32 hex digits>_<random part>`: it names its tenant, so it works without the `Autoxide-Tenant` header or the tenant's domain. Keys created by older versions start with `axk_` and keep working.

<!-- example: use-key -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $KEY"
```

<!-- example: beyond-its-permissions -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $KEY" \
  --json '{"kind":"post","title":"Not allowed"}'
# expect: 403
```

Requests with an API key have their own rate limit (`AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE`, 120 by default). Past it the answer is `429` with `Retry-After`; see [errors](errors.md).

## List and revoke

<!-- example: list-keys -->
```bash
curl -sS "$API/api/v1/auth/api-keys" -H "Authorization: Bearer $TOKEN"
```

<!-- example: revoke-key -->
```bash
curl -sS -X DELETE "$API/api/v1/auth/api-keys/$KEY_ID" -H "Authorization: Bearer $TOKEN"
# expect: 204
```

<!-- example: revoked-key-is-refused -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $KEY"
# expect: 401
```

## Every user's keys (administrators)

With the permission `auth.api_keys.manage`, `GET /api/v1/auth/all-api-keys` (filter `user_id=`) lists the keys of every user with their owner's name and email, never their secrets, and `DELETE /api/v1/auth/all-api-keys/{id}` revokes any of them — e.g. when a key leaks or its owner leaves. The audit log records who revoked it.

## With the SDK

```ts
import { createClient } from "@autoxide/sdk";

// A build script. Never write the key in the code: read it from your secret store.
const api = createClient({ baseUrl: "https://api.example.com", token: "axk1_…" /* read it from your secret store */ });

for await (const entry of api.paginate("/api/v1/content/entries", { query: { status: "published" } })) {
  console.log(entry.title);
}
```
