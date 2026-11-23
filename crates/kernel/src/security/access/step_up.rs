//! Re-confirming identity for sensitive actions (step 54 P5, "sudo mode" like GitHub; RFC 9470;
//! D-20261119-step54-step-up).
//!
//! A permission with `risk` of `destructive` or `money` (high-risk WRITE actions, D-20261119-step-up-writes)
//! can be used only when the session's most recent authentication (sign-in, or `POST /api/v1/auth/me/reauth`)
//! is still within the window. The authenticator (module `auth`) attaches that time and the window to the
//! `Principal` with [`Principal::with_step_up`]; a caller without it (API key, job, test, another
//! authenticator) is not asked.

use super::{Permission, Principal, Risk};
use crate::error::Error;
use crate::time::Timestamp;
use jiff::SignedDuration;

/// Time of the session's most recent authentication and the window for sensitive actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepUp {
    /// Most recent sign-in or re-confirmation; `None`: unknown (an old token), treated as stale.
    pub authenticated_at: Option<Timestamp>,
    /// How long after that time sensitive actions can still be done.
    pub window: SignedDuration,
}

impl Principal {
    /// Requires re-confirmation for sensitive actions according to `step_up`.
    #[must_use]
    pub fn with_step_up(mut self, step_up: StepUp) -> Self {
        self.step_up = Some(step_up);
        self
    }

    /// The attached authentication time and window, if any.
    pub fn step_up(&self) -> Option<StepUp> {
        self.step_up
    }

    /// `Ok` when `permission` does not need re-confirmation, or the most recent authentication is still
    /// within the window; otherwise 401 `step_up_required` with a `WWW-Authenticate` header per RFC 9470.
    pub(super) fn fresh_for(&self, permission: &Permission) -> Result<(), Error> {
        if !matches!(permission.risk(), Risk::Destructive | Risk::Money) {
            return Ok(());
        }
        self.require_fresh(permission)
    }

    /// Requires re-confirmation for ONE specific action even though its `permission` is not high-risk, e.g.
    /// setting a provider account (changes where money goes) under `kernel.settings.manage`
    /// (D-20261122-provider-environments). A caller without [`StepUp`] attached (API key, job) is not asked,
    /// like [`require`](Self::require).
    pub fn require_fresh(&self, permission: &Permission) -> Result<(), Error> {
        let Some(step_up) = self.step_up else { return Ok(()) };
        let fresh =
            step_up.authenticated_at.is_some_and(|at| Timestamp::now() - at <= step_up.window);
        if fresh {
            return Ok(());
        }
        let seconds = step_up.window.as_secs().max(0);
        Err(Error::unauthenticated(
            "step_up_required",
            "`{permission}` is a sensitive permission: confirm it is you again (passkey, \
             authenticator code or password) with `POST /api/v1/auth/me/reauth`, then retry \
             with the access token it returns. A confirmation lasts {minutes} minutes.",
        )
        .with_param("permission", permission.name())
        .with_param("minutes", seconds / 60)
        .with_challenge(format!(
            "Bearer error=\"insufficient_user_authentication\", error_description=\"A recent \
             sign-in is required\", max_age={seconds}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DROP: Permission =
        Permission::new("demo.drop", "Delete everything.").with_risk(Risk::Destructive);
    const READ: Permission = Permission::new("demo.read", "Read.");

    #[test]
    fn viec_nhay_cam_doi_xac_thuc_lai() {
        let window = SignedDuration::from_mins(15);
        let fresh = Principal::with_all_permissions(uuid::Uuid::now_v7())
            .with_step_up(StepUp { authenticated_at: Some(Timestamp::now()), window });
        assert!(fresh.require(&DROP).is_ok());

        let stale = Principal::with_all_permissions(uuid::Uuid::now_v7()).with_step_up(StepUp {
            authenticated_at: Some(Timestamp::now() - SignedDuration::from_mins(16)),
            window,
        });
        let error = stale.require(&DROP).unwrap_err();
        assert_eq!(error.code(), "step_up_required");
        assert_eq!(error.kind(), crate::error::ErrorKind::Unauthenticated);
        assert!(error.challenge().unwrap().contains("max_age=900"));
        assert!(stale.require(&READ).is_ok(), "việc thường không đòi");
        const PERSONAL: Permission =
            Permission::new("demo.people", "See people.").with_risk(Risk::SensitiveData);
        assert!(stale.require(&PERSONAL).is_ok(), "đọc dữ liệu nhạy cảm không đòi");
        let unknown = Principal::with_all_permissions(uuid::Uuid::now_v7())
            .with_step_up(StepUp { authenticated_at: None, window });
        assert!(unknown.require(&DROP).is_err());
        // Không gắn (API key, job): không đòi.
        assert!(Principal::with_all_permissions(uuid::Uuid::now_v7()).require(&DROP).is_ok());
        // Thiếu permission vẫn là `permission_denied`, không phải đòi xác nhận lại.
        let none = Principal::new(uuid::Uuid::now_v7(), [])
            .with_step_up(StepUp { authenticated_at: None, window });
        assert_eq!(none.require(&DROP).unwrap_err().code(), "permission_denied");
    }
}
