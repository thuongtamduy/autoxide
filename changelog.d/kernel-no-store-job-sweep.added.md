- Kernel, for module authors: `http::NoStore<T>` wraps a response that carries a secret shown once (a new API key, a
  signing secret) with `Cache-Control: no-store`, so proxies do not keep it and the idempotency layer does not store
  it; `JobContext::all_tenants(reason)` gives a job of the whole database (a cleanup schedule) `db` handles that sweep
  the rows of every tenant on that database, counted in `kernel_all_tenant_queries{module}` like
  `ControlPlane::all_tenants`.
