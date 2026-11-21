- `AUTOXIDE_HTTP_METRICS_ADDRESS`: serve `/metrics` (which has no authentication) on its own port
  that only the metrics collector reaches; `/metrics` then leaves the API port. Metrics stay off by
  default.
