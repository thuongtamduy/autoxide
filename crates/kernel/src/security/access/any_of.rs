//! "Has at least one of these permissions", by name — for the shared permission check of the HTTP layer, which
//! reads the `x-permissions` list of the OpenAPI document (D-20261126-permission-gate).

use super::Principal;
use crate::error::Error;

impl Principal {
    /// `Ok` when the caller has at least one of `names` (at any depth); otherwise 403 `permission_denied`
    /// naming the first one, with reason `denied_by_role` / `outside_guardrail` when a role or the guardrail
    /// removed it. The service still checks the depth and the record.
    pub(crate) fn require_any_named(&self, names: &[String]) -> Result<(), Error> {
        if names.iter().any(|name| self.allows(name)) {
            return Ok(());
        }
        let first = names.first().map(String::as_str).unwrap_or_default();
        let reason = self.denied.get(first).copied().unwrap_or("missing");
        Err(Error::forbidden(
            "permission_denied",
            "This action needs the permission `{permission}`. Ask an administrator for a role that \
             has it.",
        )
        .with_param("permission", first)
        .with_reason(reason))
    }
}
