- The HTTP server closes a connection whose request headers are not complete within 30 seconds, and
  pings idle HTTP/2 connections, so slow clients cannot hold connections open forever.
- `GET /api/v1/kernel/find` rejects a `q` longer than 200 characters (`422`, field `q`, code
  `length`).
- The server logs a warning at startup when rate limiting is off, when `AUTOXIDE_HTTP_CORS_ORIGINS`
  is `*`, and when `/metrics` is served on the public port.
- Connector HTTP clients for tenant-chosen addresses ignore the system proxy, so the public-address
  filter cannot be bypassed through it.
- The printable invoice HTML (`GET /api/v1/invoices/documents/{id}/render`) carries its own sandbox
  `Content-Security-Policy`, even when `AUTOXIDE_HTTP_SECURITY_HEADERS` is off.
- The startup log names the first administrator by user ID instead of email.
