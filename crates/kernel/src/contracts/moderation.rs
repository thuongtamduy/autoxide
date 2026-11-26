//! Temporarily hiding a target because of a legal case (step 37, D-363): intellectual property infringement
//! reports, counterfeit goods, complaints, requests from competent authorities.
//!
//! The case-handling module (`compliance`) does **not** touch another module's tables (architecture rule 1):
//! it emits the events [`TARGET_RESTRICTED`] / [`TARGET_RESTORED`] with a [`Restriction`] payload (a target
//! of the form `<module>.<kind>:<id>` from the target registry, D-123). The module that owns the target
//! listens to these two events through `Module::subscriptions` and hides / restores it itself (e.g. `content`
//! returns 451, `comments` hides the comment); it reads the data with [`restriction`]. Events are redelivered
//! until the receiver finishes, so the receiver must be idempotent (hiding twice behaves like hiding once).
//!
//! Like [`USER_DELETED`](crate::privacy::USER_DELETED), this is a kernel contract: the module that owns the
//! target is subscribed to these two events even when the case-handling module is not installed.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Error;
use crate::event::Delivered;
use crate::tenancy::TenantId;

/// Target is temporarily hidden (pending case handling, or the case concluded there is a violation).
pub const TARGET_RESTRICTED: &str = "compliance.target_restricted";

/// Target is restored (case dismissed, withdrawn, or a handler unhid it).
pub const TARGET_RESTORED: &str = "compliance.target_restored";

/// Payload of [`TARGET_RESTRICTED`] / [`TARGET_RESTORED`]: only IDs and kinds, never the reporter's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Restriction {
    /// The case that caused the change.
    pub case_id: Uuid,
    /// Target, in the form `<module>.<kind>:<id>` (e.g. `content.entry:0190...`).
    pub target: String,
    /// Kind of case (`ip_infringement`, `counterfeit`, `prohibited_goods`, `consumer_complaint`,
    /// `other`).
    pub case_kind: String,
}

impl Restriction {
    /// Restriction of `target` because of case `case_id` of kind `case_kind`.
    pub fn new(case_id: Uuid, target: impl Into<String>, case_kind: impl Into<String>) -> Self {
        Restriction { case_id, target: target.into(), case_kind: case_kind.into() }
    }

    /// ID of the target when it is of kind `kind` (e.g. `content.entry`); `None` for any other kind.
    pub fn id_in(&self, kind: &str) -> Option<&str> {
        self.target
            .strip_prefix(kind)
            .and_then(|rest| rest.strip_prefix(':'))
            .filter(|id| !id.is_empty())
    }
}

/// A change an event carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RestrictionChange {
    /// Hide the target.
    Restricted,
    /// Restore the target.
    Restored,
}

/// The change and payload of a [`TARGET_RESTRICTED`] / [`TARGET_RESTORED`] event; `None` when the event is
/// different or the payload has the wrong shape.
pub fn restriction(event: &Delivered) -> Option<(RestrictionChange, Restriction)> {
    let change = match event.name.as_str() {
        TARGET_RESTRICTED => RestrictionChange::Restricted,
        TARGET_RESTORED => RestrictionChange::Restored,
        _ => return None,
    };
    serde_json::from_value(event.payload.clone()).ok().map(|payload| (change, payload))
}

/// Consumer content moderation rule (step 37, D-364; Law on Protection of Consumers' Rights 19/2023/QH15,
/// Decree 55/2024/ND-CP — check against the original text): a platform must not hide reviews contrary to the
/// rules. Modules that hide buyer reviews / content (e.g. reviews of `orders`) ask before hiding and record the reason in
/// their own event. The list of reasons is the rule `moderation.hide_reasons` (data with effective dates).
/// Published by the `compliance` module; when it is not installed, modules fall back to their own reason
/// lists.
#[async_trait::async_trait]
pub trait ModerationPolicy: Send + Sync {
    /// The reason codes allowed for hiding at `surface` (e.g. `reviews`, `comments`).
    async fn hide_reasons(&self, tenant: TenantId, surface: &str) -> Result<Vec<String>, Error>;

    /// Whether `reason` is allowed at `surface`; if not → `compliance.hide_reason_not_allowed`
    /// (422, with `allowed`).
    async fn check_hide_reason(
        &self,
        tenant: TenantId,
        surface: &str,
        reason: &str,
    ) -> Result<(), Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_cua_dich_theo_loai() {
        let id = Uuid::now_v7();
        let restriction = Restriction::new(id, format!("content.entry:{id}"), "counterfeit");
        assert_eq!(restriction.id_in("content.entry"), Some(id.to_string()).as_deref());
        assert_eq!(restriction.id_in("content.term"), None);
        assert_eq!(restriction.id_in("content"), None);
        assert_eq!(Restriction::new(id, "content.entry:", "other").id_in("content.entry"), None);
    }
}
