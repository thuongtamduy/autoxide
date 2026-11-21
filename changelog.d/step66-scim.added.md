- Module `auth`: SCIM 2.0 endpoint at `/api/v1/auth/scim/v2` (`Users`, `ServiceProviderConfig`,
  `ResourceTypes`, `Schemas`; filters `eq`/`ne`/`co`/`sw`/`ew`/`pr`, PATCH per RFC 7644) so Microsoft
  Entra ID, Okta and Google Workspace can create, change and disable staff accounts. `active: false`
  disables the account and revokes its sessions, refresh tokens and API keys in the same transaction.
  Tenant SCIM tokens are managed at `/api/v1/auth/scim/tokens` (permission `auth.scim.manage`, plan
  feature `auth.scim`). Guide: `docs/guides/scim.md`.
- Kernel: `autoxide_kernel::http::StandardErrorBody` keeps an error body that follows another
  standard (e.g. SCIM errors) instead of turning it into Problem Details.
