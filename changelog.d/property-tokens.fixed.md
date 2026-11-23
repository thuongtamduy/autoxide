- Sign-in with an OpenID Connect provider: an `id_token` whose `exp`, `iat` or `nbf` claim is an extreme number no
  longer crashes the request (500); the time checks saturate instead of overflowing.
- Property tests now try to break access tokens, API keys, the `Authorization` header, OIDC `id_token` parsing, SCIM
  filters, ALTCHA solutions, `Accept-Language` and the media `Range` header.
