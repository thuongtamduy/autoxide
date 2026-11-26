- Sign-in policy per tenant (organization settings at the tenant's root unit): `auth.staff_registration`
  (self-registration of staff, also for a first OIDC sign-in), `auth.password_min_length`,
  `auth.session_days` and `auth.totp_issuer`. A tenant can only make the server's policy stricter: a
  shorter password minimum or a longer session than the server's is ignored.
- Passkeys per tenant: tenant setting `auth.passkey_rp_id`, one of the tenant's verified domains, checked
  when set and at every use. Kernel: `TenantSetting::own_domain` and `SiteLinks::own_domain` for any
  setting whose value must be a domain the tenant has verified. `GET /api/v1/kernel/client-config` gives
  `auth.passkeys`, `auth.passkey_rp_id` and `auth.self_registration` for the request's tenant.
