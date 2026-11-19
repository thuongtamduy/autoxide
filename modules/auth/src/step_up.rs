//! Xác nhận lại cho việc nhạy cảm (bước 54 P5, "sudo mode" như GitHub; D-20261119-step54-step-up):
//! phiên đăng nhập chỉ dùng được permission `destructive` / `money` / `sensitive_data` trong
//! `AUTOXIDE_AUTH_STEP_UP_MINUTES` phút sau lần đăng nhập hay `POST /me/reauth` gần nhất. Bật mặc định;
//! khách tắt bằng cấu hình `auth.step_up` (có audit như mọi cấu hình theo khách). API key không bị
//! đòi (theo đặc tả 54: việc nhạy cảm của agent đi đường duyệt).

use std::sync::Arc;

use autoxide_kernel::access::{Principal, StepUp};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{TenantContext, TenantSetting, TenantSettings, parsed_setting};
use chrono::{DateTime, Duration, Utc};

/// Cấu hình theo khách: đòi xác nhận lại cho việc nhạy cảm.
pub(crate) const STEP_UP: TenantSetting = TenantSetting::new(
    "auth.step_up",
    "`true` (default): sensitive actions (deleting for good, money, other people's personal data) \
     need a sign-in or confirmation (`POST /api/v1/auth/me/reauth`) from the last few minutes \
     (`AUTOXIDE_AUTH_STEP_UP_MINUTES`). `false`: any valid session may do them.",
)
.checked(check_flag);

fn check_flag(raw: &str) -> Result<(), FieldError> {
    parse_flag(raw).map(|_| ())
}

fn parse_flag(raw: &str) -> Result<bool, FieldError> {
    match raw.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(FieldError::new("value", "invalid", "must be `true` or `false`")),
    }
}

/// Chính sách xác nhận lại của chương trình.
pub(crate) struct StepUpPolicy {
    /// Cửa sổ sau lần xác thực (`AUTOXIDE_AUTH_STEP_UP_MINUTES`).
    pub window: Duration,
    /// Cấu hình theo khách; `None` (chương trình rút gọn): luôn bật.
    pub settings: Option<Arc<dyn TenantSettings>>,
}

impl StepUpPolicy {
    /// Gắn đòi hỏi xác nhận lại vào người gọi bằng phiên đăng nhập, trừ khi khách đã tắt.
    pub(crate) async fn apply(
        &self,
        tenant: &TenantContext,
        principal: Principal,
        authenticated_at: Option<DateTime<Utc>>,
    ) -> Result<Principal, Error> {
        if let Some(settings) = &self.settings
            && parsed_setting(settings.as_ref(), tenant, &STEP_UP, parse_flag).await? == Some(false)
        {
            return Ok(principal);
        }
        Ok(principal.with_step_up(StepUp { authenticated_at, window: self.window }))
    }
}
