//! Xác nhận lại danh tính cho việc nhạy cảm (bước 54 P5, "sudo mode" như GitHub; RFC 9470;
//! D-20261119-step54-step-up).
//!
//! Permission có `risk` `destructive` hay `money` (việc GHI rủi ro cao, D-20261119-step-up-writes) chỉ dùng được khi lần xác thực
//! gần nhất của phiên (đăng nhập, hay `POST /api/v1/auth/me/reauth`) còn trong cửa sổ. Bộ xác thực
//! (module `auth`) gắn thời điểm đó và cửa sổ vào `Principal` bằng [`Principal::with_step_up`];
//! người gọi không gắn (API key, job, test, bộ xác thực khác) thì không bị đòi.

use super::{Permission, Principal, Risk};
use crate::error::Error;
use crate::time::Timestamp;
use jiff::SignedDuration;

/// Thời điểm xác thực gần nhất của phiên và cửa sổ cho việc nhạy cảm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepUp {
    /// Lần đăng nhập hay xác nhận lại gần nhất; `None`: không biết (token cũ) — coi như đã cũ.
    pub authenticated_at: Option<Timestamp>,
    /// Bao lâu sau lần đó thì việc nhạy cảm còn làm được.
    pub window: SignedDuration,
}

impl Principal {
    /// Đòi xác nhận lại cho việc nhạy cảm theo `step_up`.
    #[must_use]
    pub fn with_step_up(mut self, step_up: StepUp) -> Self {
        self.step_up = Some(step_up);
        self
    }

    /// Thời điểm xác thực và cửa sổ đã gắn, nếu có.
    pub fn step_up(&self) -> Option<StepUp> {
        self.step_up
    }

    /// `Ok` khi `permission` không cần xác nhận lại, hay lần xác thực gần nhất còn trong cửa sổ;
    /// không thì 401 `step_up_required` kèm `WWW-Authenticate` của RFC 9470.
    pub(super) fn fresh_for(&self, permission: &Permission) -> Result<(), Error> {
        let Some(step_up) = self.step_up else { return Ok(()) };
        if !matches!(permission.risk(), Risk::Destructive | Risk::Money) {
            return Ok(());
        }
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
