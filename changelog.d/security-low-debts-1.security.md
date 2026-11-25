- The API documentation page (`/api/docs`) allows only the pinned Scalar script file in its
  Content Security Policy, not every file on the CDN; styles and fonts no longer come from the CDN
  (L-9 of the 2026-11 security review).
- The HTTP server keeps at most `AUTOXIDE_HTTP_MAX_CONNECTIONS` connections open (default `4096`,
  `0`: no limit); over it new connections wait instead of exhausting file descriptors. A connection
  that sends nothing at all is closed after 30 seconds, like a slow request header (L-10).
- Responses from outside providers (AI, payment, shipping, SMS, e-invoice) are read up to
  `AUTOXIDE_PROVIDER_MAX_RESPONSE_BYTES` (default 16 MiB); a larger answer fails the call as if the
  provider were unreachable (I-2).
- CI and `gate.sh` run the `gitleaks` image pinned by digest, from one place
  (`.github/gitleaks/Dockerfile`) that Dependabot updates (I-5).
