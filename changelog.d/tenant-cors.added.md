- CORS follows tenants' domains: in multi-tenant mode a page at `https://<domain>` of a tenant's
  verified domain may call the API from a browser without listing it in
  `AUTOXIDE_HTTP_CORS_ORIGINS` and without a restart, but only for that tenant (otherwise
  `403 tenant_origin_mismatch`); the MCP endpoint's `Origin` check uses the same rule. A removed
  domain loses it within `AUTOXIDE_TENANCY_CACHE_SECONDS`.
