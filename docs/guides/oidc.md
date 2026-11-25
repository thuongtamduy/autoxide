# Signing in with Google, Microsoft, GitHub or OpenID Connect

Vietnamese: [`oidc.vi.md`](oidc.vi.md). Examples use `curl` with `API` set to your server (`export API=https://api.example.com`). They run as tests against every build, so they match the server of this version.

People can sign in with an outside identity provider instead of a password: Google, Microsoft (Entra ID), GitHub, or any OpenID Connect issuer. Each tenant adds its own providers. Autoxide is only the *client* of the provider; your interface drives the browser.

It is **off by default**. The server operator turns it on with `AUTOXIDE_AUTH_OIDC=true`. Providers can be managed while it is off; signing in then answers `404 auth.oidc_disabled`.

## 1. Register the application at the provider

Register a web application at the provider and note its client id and client secret. Give it your interface's callback page as the redirect (return) address, e.g. `https://app.example.com/sign-in/callback`.

- **Google** — Google Cloud console, *APIs & Services → Credentials → OAuth client ID* (type *Web application*). Scopes `openid email profile`.
- **Microsoft** — Microsoft Entra admin center, *App registrations → New registration*, platform *Web*. Create a client secret under *Certificates & secrets*. To trust the email of work accounts, add the optional claim `xms_edov` to the ID token (*Token configuration*): without it, Autoxide never links by email for Microsoft, because Entra lets the email be changed without verifying it.
- **GitHub** — *Settings → Developer settings → OAuth Apps → New OAuth App*. GitHub has no OpenID Connect: Autoxide reads the user's id and their primary, verified email from the GitHub API.
- **Any OpenID Connect issuer** (Keycloak, Auth0, Okta, Zitadel…) — the issuer must publish `/.well-known/openid-configuration` and sign ID tokens with `RS256` or `ES256`.

## 2. Add the provider to the tenant

Needs the permission `auth.oidc.manage` (`auth.oidc.read` to only list them) and, to add or change a provider, a signed-in session: an API key cannot (`auth.session_required`). The client secret is stored encrypted and never shown again (`has_secret` tells whether one is stored).

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

<!-- example: add-provider -->
```bash
curl -sS "$API/api/v1/auth/oidc-providers" -H "Authorization: Bearer $TOKEN" \
  --json '{"slug":"google","kind":"google","name":"Google","client_id":"1234.apps.googleusercontent.com","client_secret":"from-google","redirect_uris":["https://app.example.com/sign-in/callback"]}'
# expect: 201
```

- `kind` `oidc` needs `issuer`. `microsoft` defaults to any work or school account (`https://login.microsoftonline.com/organizations/v2.0`); give `https://login.microsoftonline.com/<tenant id>/v2.0` to allow only your company. `github` with an `issuer` is a GitHub Enterprise Server.
- `redirect_uris`: the exact pages the provider may send the browser back to. They are compared character by character.
- `allowed_email_domains`: only these email domains may sign in.
- `allow_signup`: someone without an account gets one on first sign-in — only if self-registration is open too (`AUTOXIDE_AUTH_SELF_REGISTRATION` for staff, the setting `auth.customer_registration` for `realm` `customer`), **or** the provider gives new accounts a role (`default_role_id`, at `default_org_id`), which is how a company lets its staff in with its own Entra ID. Whoever saves a provider that gives a role must be able to give that role themselves, on every save. At each sign-up the server checks again that the person who last saved the provider is still active, still holds `auth.oidc.manage` and can still give that role; otherwise sign-up is refused (`auth.oidc_default_role_unavailable`) until an administrator saves the provider again.
- The provider address must be public: addresses on private networks are refused unless the operator sets `AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS=true` (for tests only).

Change it with `PATCH /api/v1/auth/oidc-providers/{id}` and remove it with `DELETE` (every account link to it goes too).

## 3. Sign in

The interface lists the buttons from `GET /api/v1/auth/oidc/providers` (public). This example server has outside sign-in off:

<!-- example: list-providers -->
```bash
curl -sS "$API/api/v1/auth/oidc/providers"
# expect: 404
```

Then, with outside sign-in on:

1. `POST /api/v1/auth/oidc/{slug}/start` with `{"redirect_uri": "https://app.example.com/sign-in/callback"}` answers `authorization_url`. Open it in the browser. The server keeps the PKCE verifier, `state` and `nonce` for 10 minutes; they never leave it.
2. The provider sends the browser back to your page with `code` and `state` in the query.
3. `POST /api/v1/auth/oidc/{slug}/callback` with `{"code": "…", "state": "…"}` answers exactly like `POST /api/v1/auth/login`: tokens, or `202` with an `mfa_token` when the account has two-step sign-in (continue with `POST /api/v1/auth/login/mfa`). The `state` works once.

Which account signs in:

- the account already linked to this provider identity;
- otherwise the account with the same email — **only when both the provider and Autoxide have verified it**; if either has not, the answer is `409 auth.oidc_link_required`: sign in the usual way and link the provider (below). This stops someone from taking over an account created in advance with another person's email;
- otherwise a new account, when allowed (see `allow_signup`), else `403 auth.oidc_signup_closed`. An unverified email never creates an account (`403 auth.oidc_email_unverified`).

Errors: `400 auth.oidc_state_invalid` (start again), `401 auth.oidc_token_invalid` with `reason` (`signature`, `issuer`, `audience`, `expired`, `nonce`, `unknown_key`, `algorithm`…), `403 auth.oidc_email_domain_not_allowed`, `503 auth.oidc_provider_unavailable`.

## 4. Link and unlink in the account

- `GET /api/v1/auth/me/identities` lists the linked providers.
- `POST /api/v1/auth/me/identities/{slug}/start` with `redirect_uri` and your `current_password` (or a `reauth_token` from `POST /api/v1/auth/me/reauth`), then `POST /api/v1/auth/me/identities/{slug}/callback` with `code` and `state`, signed in with the same session.
- `DELETE /api/v1/auth/me/identities/{id}` removes a link, but not your last way to sign in (`409 auth.identity_last_method`): set a password or add a passkey first.

Linked identities (provider subject and email) are part of the account's personal data export and are deleted with the account.

Signing in with Google or GitHub does not replace the phone-number verification some modules require for posting content (Vietnamese Decree 147/2024/ND-CP).
