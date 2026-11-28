- `autoxide-testkit`: a single-tenant `TestApp` now builds its database registry from the configuration it
  read, like the real binary, so `AUTOXIDE_TENANCY_*` settings set with `TestAppBuilder::setting` take effect
  (they were silently ignored outside `multi()`; e.g. `AUTOXIDE_TENANCY_CACHE_SECONDS` stayed at 10 seconds).
