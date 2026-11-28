- HTTP: responses carry `RateLimit-Policy` and `RateLimit` (draft-ietf-httpapi-ratelimit-headers-12): policy
  `address` on every counted response, `api-key` and `tenant` on their 429. The OpenAPI document declares a 429
  response (shared `components.responses.TooManyRequests`, with `Retry-After` and both headers) on every operation
  except health checks.
