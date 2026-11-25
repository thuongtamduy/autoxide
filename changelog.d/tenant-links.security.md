- `kernel.public_url` and mail link settings no longer accept another tenant's verified domain
  (`422 kernel.public_url_not_allowed` / `kernel.link_host_not_allowed`), nor a domain the tenant
  lost (revoked, or failed its DNS recheck) until it is verified again. When a domain is revoked, fails
  or moves to another tenant, mail links stop going there — on other servers within
  `AUTOXIDE_TENANCY_CACHE_SECONDS` (default 10 seconds).
