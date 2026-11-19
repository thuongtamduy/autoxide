- Sensitive actions (permissions with `risk` `destructive` or `money`) need a sign-in or a
  confirmation from the last `AUTOXIDE_AUTH_STEP_UP_MINUTES` minutes (15 by default). Otherwise the answer is 401
  `step_up_required` with an RFC 9470 `WWW-Authenticate` challenge; `POST /api/v1/auth/me/reauth` now also returns a
  fresh `access_token` to retry with. On by default; a tenant turns it off with the setting `auth.step_up`. API keys
  are not asked.
