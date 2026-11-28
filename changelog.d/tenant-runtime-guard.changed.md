- Kernel: `EntitlementsHandle::count_request` and `request_quota_wait` take a `tenancy::SignedInTenant`, built only
  from an authenticated `Principal` (`SignedInTenant::of`), instead of a `TenantId`: a tenant named by an
  unauthenticated request (`Autoxide-Tenant`, `Host`) cannot be passed to the plan quota by mistake.
