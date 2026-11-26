//! "Has at least one of these permissions": for the shared permission check of the HTTP layer, which reads the
//! `x-permissions` list of the OpenAPI document (D-20261126-permission-gate), and for services that accept any of
//! several permissions.

use super::{Permission, Principal};
use crate::error::Error;

impl Principal {
    /// `Ok` when the caller has at least one of `permissions` (at any depth) and, like [`Principal::require`], has
    /// re-confirmed the sign-in recently when that permission is high-risk (step-up); otherwise 403
    /// `permission_denied` naming every one of them, with reason `denied_by_role` / `outside_guardrail` when a role
    /// or the guardrail removed the first. The service still checks the depth and the record.
    pub fn require_any(&self, permissions: &[&Permission]) -> Result<(), Error> {
        // Bước xác nhận lại theo permission ĐẦU TIÊN người gọi có, như `require` của permission đó.
        if let Some(held) = permissions.iter().find(|permission| self.has(permission)) {
            return self.fresh_for(held);
        }
        Err(self.refuse_any(permissions.iter().map(|permission| permission.name())))
    }

    /// "Has at least one of `names`", by name, for the HTTP permission layer only: it does not ask for step-up —
    /// the service's own `require` of the permission it uses does.
    pub(crate) fn require_any_named(&self, names: &[String]) -> Result<(), Error> {
        if names.iter().any(|name| self.allows(name)) {
            return Ok(());
        }
        Err(self.refuse_any(names.iter().map(String::as_str)))
    }

    fn refuse_any<'a>(&self, mut names: impl Iterator<Item = &'a str>) -> Error {
        let first = names.next().unwrap_or_default();
        let others: Vec<&str> = names.collect();
        if others.is_empty() {
            return self.refuse_named(first);
        }
        let reason = self.denied.get(first).copied().unwrap_or("missing");
        // Ngăn bằng "`, `" để danh sách vẫn nằm gọn trong cặp dấu `…` của câu (cả bản dịch).
        let listed = std::iter::once(first).chain(others).collect::<Vec<_>>().join("`, `");
        Error::forbidden(
            "permission_denied",
            "This action needs one of the permissions `{permission}`. Ask an administrator for a role \
             that has one of them.",
        )
        .with_param("permission", listed)
        .with_reason(reason)
    }

    fn refuse_named(&self, name: &str) -> Error {
        let reason = self.denied.get(name).copied().unwrap_or("missing");
        Error::forbidden(
            "permission_denied",
            "This action needs the permission `{permission}`. Ask an administrator for a role that \
             has it.",
        )
        .with_param("permission", name)
        .with_reason(reason)
    }
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;

    use super::super::{Risk, StepUp};
    use super::*;
    use crate::time::Timestamp;

    const DROP: Permission =
        Permission::new("demo.drop", "Delete everything.").with_risk(Risk::Destructive);
    const DROP_OWN: Permission = Permission::new("demo.drop_own", "Delete your own.");
    const READ: Permission = Permission::new("demo.read", "Read.");

    #[test]
    fn mot_trong_cac_quyen_van_doi_xac_nhan_lai_cho_viec_nhay_cam() {
        let window = SignedDuration::from_mins(15);
        let stale = StepUp {
            authenticated_at: Some(Timestamp::now() - SignedDuration::from_mins(16)),
            window,
        };
        // Có quyền huỷ (Destructive) mà phiên cũ: như `require`, phải xác nhận lại.
        let destroyer =
            Principal::new(uuid::Uuid::now_v7(), ["demo.drop".to_owned()]).with_step_up(stale);
        assert_eq!(
            destroyer.require_any(&[&DROP, &DROP_OWN]).unwrap_err().code(),
            "step_up_required"
        );
        let fresh = StepUp { authenticated_at: Some(Timestamp::now()), window };
        let destroyer =
            Principal::new(uuid::Uuid::now_v7(), ["demo.drop".to_owned()]).with_step_up(fresh);
        assert!(destroyer.require_any(&[&DROP, &DROP_OWN]).is_ok());
        // Chỉ có quyền thường trong danh sách: không đòi.
        let own =
            Principal::new(uuid::Uuid::now_v7(), ["demo.drop_own".to_owned()]).with_step_up(stale);
        assert!(own.require_any(&[&DROP, &DROP_OWN]).is_ok());
        // Không có quyền nào: `permission_denied` nêu đủ các quyền.
        let reader = Principal::new(uuid::Uuid::now_v7(), ["demo.read".to_owned()]);
        let error = reader.require_any(&[&DROP, &DROP_OWN]).unwrap_err();
        assert_eq!(error.code(), "permission_denied");
        assert!(error.message().contains("`demo.drop`, `demo.drop_own`"), "{}", error.message());
        assert!(reader.require_any(&[&READ]).is_ok());
    }
}
