- Module `auth`: reporting "this wasn't me" after an email change now also removes linked external
  sign-ins and revokes pending sign-in links and re-confirmation tokens, so an attacker who changed
  the email cannot get back in that way.
- Module `auth`: `AUTOXIDE_AUTH_TOKEN_SECRET` and `AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS` refuse the
  example value from `.env.example` at startup, like `AUTOXIDE_SECRET_KEY`.
- Module `auth`: a role that requires two-step sign-in (`requires_mfa`) no longer grants its
  permissions in another tenant, through a delegation or a support access grant, to a user who has
  not turned two-step sign-in on — the same rule as in their own tenant.
- Module `auth`: an account that holds a role requiring two-step sign-in and whose only second step
  is a passkey must sign in with the passkey; password, sign-in link and external sign-in are refused
  with `403 auth.passkey_required` (before, a password alone opened a session with the role's
  permissions).
