- The sign-in guide said two-step sign-in answers a `challenge_id`; it answers an `mfa_token` (`challenge_id` is
  for signing in with a phone number). It now also says a TOTP code is accepted only once.
- The mobile guide said to upload through `POST /api/v1/media` as a multipart form; the endpoint takes the file
  itself as the body (a multipart form is refused with `415`).
