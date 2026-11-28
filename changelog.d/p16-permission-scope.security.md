- Tenant settings (`PUT` / `DELETE /api/v1/kernel/settings/{name}`, tools `kernel.set_setting` and
  `kernel.delete_setting`) need `kernel.settings.manage` held at the tenant's root unit: a role held at a branch, or an
  API key bound to a branch, now gets `403 org_out_of_scope` instead of changing settings such as
  `kernel.business_model` for the whole tenant.
- Module `auth`: lifting a ban (taking away a role that bans a permission, or removing a ban from a role) needs that
  permission for the whole tenant, not just its name at some unit; deleting, locking or changing the email of another
  account (and revoking their API keys) needs covering each of their permissions at its unit and depth, not only by
  name (`auth.target_has_more_permissions` otherwise).
