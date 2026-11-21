- Module `auth`: `POST /api/v1/auth/oidc/{slug}/start` and `POST /api/v1/auth/me/identities/{slug}/start`
  accept `bind_browser`; the returned `browser_secret` must then be sent to the callback.
