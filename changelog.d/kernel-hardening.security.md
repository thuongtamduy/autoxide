- MCP tools (and automation steps) ask for a recent sign-in confirmation for sensitive permissions, as REST does.
- `webhooks`: subscribing an endpoint to an event needs the event's `stream_permission` (`*`: all of them), as the
  event stream does.
- Tenant templates cannot use `{% autoescape %}` (`template.autoescape_forbidden`).
- `media`: uploaded SVG files may only contain known SVG elements; `<!DOCTYPE`, entities, CDATA and processing
  instructions are refused. New setting `AUTOXIDE_MEDIA_IMAGE_JOBS` (default 2) limits how many images are decoded at
  once.
- The API reference page loads a pinned Scalar script with Subresource Integrity; the quickstart turns the page off by
  default.
- `AUTOXIDE_SECRET_KEY` set to the example value is refused; `compose.yml` requires it. A warning is logged when
  `AUTOXIDE_HTTP_CLIENT_IP_HEADER` is set without `AUTOXIDE_HTTP_TRUSTED_PROXIES`.
- Searches and slug checks escape `%` and `_` in user text.
- `automations`: a rule created, edited or enabled with an API key runs with at most that key's permissions and unit
  fence, and pauses (`credential_revoked`) when the key is revoked.
- The generated password of the first administrator is written to `data/initial-admin-password` (mode 0600), not to
  the log.
