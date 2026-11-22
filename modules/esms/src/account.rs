//! Tài khoản eSMS của TỪNG KHÁCH: bốn cấu hình theo khách nhóm `connection`
//! (D-20261121-provider-accounts). Khách đặt bằng `PUT /api/v1/kernel/settings/{name}`.

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::provider::{Environment, ProviderAccounts};
use autoxide_kernel::tenancy::{CONNECTION_GROUP, SettingValue, TenantContext, TenantSetting};

/// Kết nối mà bốn cấu hình dưới thuộc về (tên của `ConnectionProbe` và của trạng thái lần dùng cuối).
pub(crate) const CONNECTION: &str = "esms.connection.sms";

pub(crate) const API_KEY: TenantSetting = TenantSetting::new(
    "esms.connection.sms.api_key",
    "ApiKey of this tenant's eSMS account (esms.vn, after signing in: menu API management).",
)
.secret()
.group(CONNECTION_GROUP);

pub(crate) const SECRET_KEY: TenantSetting = TenantSetting::new(
    "esms.connection.sms.secret_key",
    "SecretKey of this tenant's eSMS account (same page as the ApiKey).",
)
.secret()
.group(CONNECTION_GROUP);

pub(crate) const BRANDNAME: TenantSetting = TenantSetting::new(
    "esms.connection.sms.brandname",
    "Brandname the messages are sent under. It must be registered with eSMS for customer-care \
     messages, together with the message templates, before anything is delivered.",
)
.group(CONNECTION_GROUP)
.checked(check_brandname);

// Quy ước chung của mọi bộ nối (D-20261122-provider-environments). eSMS không có host sandbox riêng:
// môi trường thử là trường `Sandbox` trong thân request tới cùng một địa chỉ, nên ở đây môi trường
// chỉ bật trường đó, không chọn địa chỉ.
pub(crate) const ENVIRONMENT: TenantSetting = TenantSetting::new(
    "esms.connection.sms.environment",
    "`sandbox`: messages go to the eSMS test environment — the account and parameters are \
     checked, nothing is delivered and nothing is charged. `production` or unset: messages are \
     really sent.",
)
.group(CONNECTION_GROUP)
.checked(Environment::check);

fn check_brandname(raw: &str) -> Result<(), FieldError> {
    let length = raw.trim().chars().count();
    if (1..=50).contains(&length) {
        Ok(())
    } else {
        Err(FieldError::new("value", "length", "must be a brandname of 1 to 50 characters"))
    }
}

/// Tài khoản đã đọc của một khách. Không `Debug`: có secret.
pub(crate) struct Account {
    pub api_key: SettingValue,
    pub secret_key: SettingValue,
    pub brandname: String,
    pub environment: Environment,
}

impl Account {
    /// Thiếu trường bắt buộc → `provider.not_configured` nêu đúng tên cấu hình còn thiếu; không bao
    /// giờ rơi về tài khoản của khách khác hay của hệ thống.
    pub(crate) async fn read(
        accounts: &ProviderAccounts,
        tenant: &TenantContext,
    ) -> Result<Self, Error> {
        let api_key = accounts.require(tenant, &API_KEY).await?;
        let secret_key = accounts.require(tenant, &SECRET_KEY).await?;
        let brandname = accounts.require(tenant, &BRANDNAME).await?.as_str().trim().to_owned();
        // Giá trị đã được kiểm lúc lưu; giá trị lạ (không thể có) coi như gửi thật thì nguy hiểm hơn
        // là báo lỗi, nên báo lỗi cấu hình.
        let stored = accounts.get(tenant, &ENVIRONMENT).await?;
        let environment = Environment::parse(stored.as_ref().map(|value| value.as_str()))
            .ok_or_else(|| autoxide_kernel::provider::not_configured(ENVIRONMENT.name()))?;
        Ok(Account { api_key, secret_key, brandname, environment })
    }
}
