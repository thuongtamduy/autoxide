//! Chính sách đăng nhập theo khách (D-20261126-tenant-login-policy): độ dài mật khẩu tối thiểu, thời hạn phiên, tên
//! trong app xác thực. Đọc ở nút gốc của khách (cài đặt của `organizations`: một lần đọc nhỏ, chỉ lúc đăng nhập, làm
//! mới phiên, đặt mật khẩu, bật TOTP); chỉ siết hơn cấu hình của máy chủ — giá trị khách đặt lỏng hơn bị bỏ qua,
//! không lỗi.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_organizations::api::OrgDirectory;
use jiff::SignedDuration;

use crate::settings;

/// Chính sách của từng khách. Rẻ khi clone. Không có `organizations` (test dựng tay): luôn theo máy chủ.
#[derive(Clone, Default)]
pub(crate) struct TenantPolicy {
    orgs: Option<Arc<dyn OrgDirectory>>,
}

// `Debug` viết tay: `dyn OrgDirectory` không có `Debug`.
impl std::fmt::Debug for TenantPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("TenantPolicy").field("orgs", &self.orgs.is_some()).finish()
    }
}

impl TenantPolicy {
    pub(crate) fn new(orgs: Option<Arc<dyn OrgDirectory>>) -> Self {
        TenantPolicy { orgs }
    }

    /// Độ dài mật khẩu tối thiểu ở khách `tenant`: lớn hơn giữa mức của máy chủ và `auth.password_min_length`.
    pub(crate) async fn password_min_length(&self, tenant: &TenantContext) -> Result<usize, Error> {
        let set =
            settings::value(self.orgs.as_ref(), tenant, &settings::PASSWORD_MIN_LENGTH).await?;
        let set =
            set.and_then(|value| value.as_u64()).and_then(|length| usize::try_from(length).ok());
        Ok(set.map_or(crate::password::MIN_LENGTH, |length| {
            length.clamp(crate::password::MIN_LENGTH, crate::password::MAX_LENGTH)
        }))
    }

    /// Thời hạn mỗi refresh token ở khách `tenant`: ngắn hơn giữa `server` và `auth.session_days`.
    pub(crate) async fn session_ttl(
        &self,
        tenant: &TenantContext,
        server: SignedDuration,
    ) -> Result<SignedDuration, Error> {
        let set = settings::value(self.orgs.as_ref(), tenant, &settings::SESSION_DAYS).await?;
        let days = set.and_then(|value| value.as_i64()).filter(|days| *days >= 1);
        Ok(days.map_or(server, |days| server.min(SignedDuration::from_hours(days * 24))))
    }

    /// Tên trong app xác thực ở khách `tenant`: `auth.totp_issuer`, không thì `server`.
    pub(crate) async fn totp_issuer(
        &self,
        tenant: &TenantContext,
        server: &str,
    ) -> Result<String, Error> {
        let set = settings::value(self.orgs.as_ref(), tenant, &settings::TOTP_ISSUER).await?;
        let name = set.as_ref().and_then(|value| value.as_str()).map(str::trim);
        Ok(match name {
            Some(name) if !name.is_empty() && !name.contains(':') => name.to_owned(),
            _ => server.to_owned(),
        })
    }
}
