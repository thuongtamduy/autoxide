//! "Has at least one of these permissions": for the shared permission check of the HTTP layer, which reads the
//! `x-permissions` list of the OpenAPI document (D-20261126-permission-gate), and for services that accept any of
//! several permissions.

use super::{Permission, Principal};
use crate::error::Error;

impl Principal {
    /// `Ok` when the caller has at least one of `permissions` (at any depth); otherwise 403 `permission_denied`
    /// naming every one of them, with reason `denied_by_role` / `outside_guardrail` when a role or the guardrail
    /// removed the first. The service still checks the depth and the record.
    pub fn require_any(&self, permissions: &[&Permission]) -> Result<(), Error> {
        self.require_any_of(permissions.iter().map(|permission| permission.name()))
    }

    /// [`Principal::require_any`] by name.
    pub(crate) fn require_any_named(&self, names: &[String]) -> Result<(), Error> {
        self.require_any_of(names.iter().map(String::as_str))
    }

    fn require_any_of<'a>(
        &self,
        names: impl Iterator<Item = &'a str> + Clone,
    ) -> Result<(), Error> {
        if names.clone().any(|name| self.allows(name)) {
            return Ok(());
        }
        let first = names.clone().next().unwrap_or_default();
        let reason = self.denied.get(first).copied().unwrap_or("missing");
        // Nêu mọi permission dùng được, ngăn bằng "`, `" để vẫn nằm gọn trong cặp dấu `…` của câu (cả bản dịch).
        let listed = names.collect::<Vec<_>>().join("`, `");
        Err(Error::forbidden(
            "permission_denied",
            "This action needs the permission `{permission}`. Ask an administrator for a role that \
             has it.",
        )
        .with_param("permission", listed)
        .with_reason(reason))
    }
}
