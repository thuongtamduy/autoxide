# Signing in and staying signed in

Vietnamese: [`authentication.vi.md`](authentication.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

## Sign in

Send the email and password. The answer holds a short-lived **access token** (send it with every request) and a **refresh token** (to get the next pair). An account with two-step sign-in answers `202` with a `challenge_id` instead; finish with `POST /api/v1/auth/login/mfa` and the code.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

<!-- example: who-am-i -->
```bash
curl -sS "$API/api/v1/auth/me" -H "Authorization: Bearer $TOKEN"
```

## Refresh before the access token expires

`expires_in` says how many seconds the access token lives. Shortly before, exchange the refresh token for a new pair. Each refresh token works **once**: keep the new one.

<!-- example: refresh -->
```bash
curl -sS "$API/api/v1/auth/refresh" --json '{"refresh_token":"'"$REFRESH"'"}'
# save: TOKEN=.data.access_token
# save: NEW_REFRESH=.data.refresh_token
```

Sending an old refresh token again looks like a stolen token: the server refuses it (`auth.refresh_token_reused`) **and ends the whole session**, so the newest refresh token stops working too. Sign in again.

<!-- example: refresh-token-reused -->
```bash
curl -sS "$API/api/v1/auth/refresh" --json '{"refresh_token":"'"$REFRESH"'"}'
# expect: 401
```

<!-- example: session-ended -->
```bash
curl -sS "$API/api/v1/auth/refresh" --json '{"refresh_token":"'"$NEW_REFRESH"'"}'
# expect: 401
```

## Sign out

Ending a session takes its refresh token; the access token stops working when it expires.

<!-- example: sign-in-again -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

<!-- example: sign-out -->
```bash
curl -sS "$API/api/v1/auth/logout" --json '{"refresh_token":"'"$REFRESH"'"}'
# expect: 204
```

## With the SDK

Give the client a function for the token, so each request reads the current one:

```ts
import { createClient } from "@autoxide/sdk";

let tokens = { access: "", refresh: "" };
const api = createClient({ baseUrl: "https://api.example.com", token: () => tokens.access });

const signedIn = await api.post("/api/v1/auth/login", {
  body: { email: "admin@example.com", password: "correct horse battery staple" },
});
if ("access_token" in signedIn) {
  tokens = { access: signedIn.access_token, refresh: signedIn.refresh_token };
}

const me = await api.get("/api/v1/auth/me");
console.log(me.email, me.permissions);

const next = await api.post("/api/v1/auth/refresh", { body: { refresh_token: tokens.refresh } });
tokens = { access: next.access_token, refresh: next.refresh_token };
```
