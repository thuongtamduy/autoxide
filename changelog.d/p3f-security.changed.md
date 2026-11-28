- Module `auth`: editing a role that removes or narrows a permission, or adds a ban, now needs that permission as
  widely as when giving it (`auth.permission_beyond_you`); a role banning a permission outside the tenant's
  guardrail can be taken away by anyone who can take away the role.
- Kernel: `Principal::in_tenant` moves grants held at the previous tenant's root to the new tenant's root, and
  `require_at_root` no longer treats the default tenant's root as the root of every tenant. New
  `Principal::outside_guardrail`. `Entitlements::confirm` without a hold returns the stable code
  `limit_hold_missing` (409) instead of an internal error.
