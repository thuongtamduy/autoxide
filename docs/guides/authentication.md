# Signing in and staying signed in

Vietnamese: [`authentication.vi.md`](authentication.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

## Sign in

Send the email and password. The answer holds a short-lived **access token** (send it with every request) and a **refresh token** (to get the next pair). An account with two-step sign-in answers `202` with a `challenge_id` instead; finish with `POST /api/v1/auth/login/mfa` and the code.

On a server that hosts several tenants, sign in on the tenant's domain or with the `Autoxide-Tenant` header. The tokens you get name their tenant, so the requests that follow — with the access token, `/refresh`, `/logout`, `/login/mfa`, and the links in password-reset and verification mails — need neither the header nor the domain. Sending a token together with a header (or domain) of another tenant is refused.

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

## Sign in with a link

When the server turns it on (`AUTOXIDE_AUTH_LOGIN_LINK=true`), `POST /api/v1/auth/login/link` with `{"email": …}` sends a one-time sign-in link (valid `AUTOXIDE_AUTH_LOGIN_LINK_MINUTES`, 15 by default). It always answers 202 the same way, whether or not the email has an account. Finish with `POST /api/v1/auth/login/link/confirm` and the `token` from the mail; the answer is the same as `/login` (tokens, or an `mfa_token` when two-step sign-in is on). Send `"bind_device": true` to get a `device_secret` and send it back with the token: the link then works only on this device.

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

## Your sessions

`GET /api/v1/auth/me/sessions` lists the devices signed in to your account: `user_agent`, `sign_in_method` (`password`, `password_and_mfa`, `passkey`, `email_link`, `password_change`) and, when the server's reverse proxy tells it (`AUTOXIDE_AUTH_COUNTRY_HEADER`, e.g. `CF-IPCountry`), the approximate `country`. The IP address is never kept. End a session you do not know with `DELETE /api/v1/auth/me/sessions/{id}`.

## Your account

`PATCH /api/v1/auth/me` changes your display name (`name`), preferred language (`locale`: `en` or `vi`) and time zone (`time_zone`, IANA, e.g. `Asia/Ho_Chi_Minh`); `null` clears the language or time zone. Mails sent later (password reset, email change notice) use your preferred language.

Sensitive changes — creating an API key, changing your email, turning on two-step sign-in, adding a passkey, deleting your account — need proof that it is you. Send your `current_password`, or first get a short-lived confirmation token with `POST /api/v1/auth/me/reauth` (`{"method": "password" | "totp" | "passkey" | "email_link", …}`) and send it as `reauth_token`. Each token works once, for `AUTOXIDE_AUTH_REAUTH_MINUTES` (10 by default). Accounts without a password use `passkey`, `totp` or `email_link`.

A role can require two-step sign-in (`requires_mfa`, also on the built-in `admin` role). Until the account turns on an authenticator app or adds a passkey, that role's permissions are left out and `GET /api/v1/auth/me` answers `mfa_required: true`.

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
