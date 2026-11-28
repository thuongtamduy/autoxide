# Signing in and staying signed in

Vietnamese: [`authentication.vi.md`](authentication.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`), and the TypeScript SDK (`sdk/typescript`). They run as tests against every build, so they match the server of this version.

## Sign in

Send the email and password. The answer holds a short-lived **access token** (send it with every request) and a **refresh token** (to get the next pair). An account with two-step sign-in answers `202` with an `mfa_token` instead (and `methods`, `expires_in`); finish with `POST /api/v1/auth/login/mfa` and `{"mfa_token": …, "code": …}`, where `code` is the 6-digit code of the authenticator app or an unused recovery code. The `mfa_token` allows a few tries within `expires_in` seconds and ends when a code is accepted; afterwards it answers `401 auth.invalid_mfa_token` (sign in again). A wrong code answers `401 auth.invalid_mfa_code`. A code that was accepted once cannot be used again within its 30-second window, nor an older one — signing in twice in a row, wait for the next code. (`challenge_id` belongs to signing in with a phone number, below.)

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

## Buyers: sign in with a phone number

Buyers can sign in, or create an account, with a code sent by SMS. It needs an SMS provider on the
server; without one these endpoints answer 503 `auth.phone_sign_in_unavailable`.

1. `POST /api/v1/auth/phone/codes` with `{"phone": "0912345678"}` answers 202 with a `challenge_id`.
   The answer is the same whether the number has an account or not.
2. `POST /api/v1/auth/phone/login` with `{"challenge_id": "…", "code": "123456", "name": "Lan"}`.
   A number that already has a buyer account signs in to it (200). A new number creates a buyer
   account without email or password (201) when the shop lets buyers create accounts; `name` is used
   only then. Both answer with the same tokens as `/login`, or 202 when the account has two-step
   sign-in on.

Such an account has `email` set to `""` until the buyer adds one. To confirm a sensitive change
(`POST /api/v1/auth/me/reauth`) it uses `"method": "phone_code"` with a fresh `phone_challenge_id`
and `phone_code` sent to its own number; after that it can add a passkey and sign in with it.

**Limits against guessing codes** (NIST SP 800-63B-4). A code lives at most 10 minutes
(`AUTOXIDE_COMPLIANCE_OTP_TTL_SECONDS`, 60 to 600) and allows a few wrong tries. Per number:

- codes per hour from one client (one IPv4 address, or one IPv6 /64) and from all clients together, and
  codes per day — sign-in codes and phone verification codes counted apart, so asking for sign-in codes
  never uses up the owner's way to verify the number again (`429 compliance.otp_throttled`);
- wrong codes per day (`429 compliance.otp_daily_limit`): for sign-in codes, from one client and from all
  clients together; for phone verification codes, only on the signed-in user's own codes for the number
  (no "all clients" cap, so nobody else's wrong codes block it);
- wrong codes in a row, with no time window: from 90 the number is locked for clients that never signed
  in with it, and from 100 for every client (`403 compliance.otp_locked`).

A client that signed in with the number in the last 90 days does not count toward the "all clients"
limits and keeps the last 10 tries of the 100, so strangers cannot lock the owner out of the networks
they use. A right code resets the count. A locked number is unlocked when the owner signs in another way
and verifies the number again in their account, or when the shop unlocks it — which also restarts the
daily limit of wrong codes, while the limits on sending codes still apply
(`POST /api/v1/compliance/phone-locks/unlock`, permission `compliance.phone_locks.manage`; MCP tool
`compliance.unlock_phone`; `autoxide task compliance.unlock_phone phone=…`) — the way back in for an
account that signs in only with its number. **Remaining risk:** mobile networks behind carrier NAT change
address often; on a network the owner never signed in from, they can be locked out when strangers reach
those limits. Offer buyers a second way to sign in (password or passkey) — NIST requires one for SMS codes.

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

Ending a session takes its refresh token. Its access tokens stop working at once (code `auth.session_ended`), without waiting for them to expire; the same happens when you end a session from the list below, change your password, or an administrator signs you out or disables the account.

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

<!-- example: signed-out-token -->
```bash
curl -sS "$API/api/v1/auth/me" -H "Authorization: Bearer $TOKEN"
# expect: 401
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

Or let the client refresh by itself: with a `refresh` function, a request that gets `401 auth.token_expired`
waits for one refresh — shared by every request that failed at the same time, since a refresh token works
once — and is sent again. [Building a web app](web-app.md) has the whole setup, with storage and several tabs.

```ts
import { createClient } from "@autoxide/sdk";

let tokens = { access: "", refresh: "" };
const signIn = createClient({ baseUrl: "https://api.example.com" });
const api = createClient({
  baseUrl: "https://api.example.com",
  token: () => tokens.access,
  refresh: async () => {
    const next = await signIn.post("/api/v1/auth/refresh", { body: { refresh_token: tokens.refresh } });
    tokens = { access: next.access_token, refresh: next.refresh_token };
  },
});

const me = await api.get("/api/v1/auth/me");
console.log(me.all_permissions || me.permissions.includes("content.write"));
```
