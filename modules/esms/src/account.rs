//! Tài khoản eSMS của TỪNG KHÁCH: bốn cấu hình theo khách nhóm `connection`
//! (D-20261121-provider-accounts). Khách đặt bằng `PUT /api/v1/kernel/settings/{name}`.

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::provider::ProviderAccounts;
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

pub(crate) const SANDBOX: TenantSetting = TenantSetting::new(
    "esms.connection.sms.sandbox",
    "`true`: messages go to the eSMS test environment — the account and parameters are checked, \
     nothing is delivered and nothing is charged. `false` or unset: messages are really sent.",
)
.group(CONNECTION_GROUP)
.checked(check_flag);

fn check_brandname(raw: &str) -> Result<(), FieldError> {
    let length = raw.trim().chars().count();
    if (1..=50).contains(&length) {
        Ok(())
    } else {
        Err(FieldError::new("value", "length", "must be a brandname of 1 to 50 characters"))
    }
}

fn check_flag(raw: &str) -> Result<(), FieldError> {
    match raw {
        "true" | "false" => Ok(()),
        _ => Err(FieldError::new("value", "invalid", "must be `true` or `false`")),
    }
}

/// Tài khoản đã đọc của một khách. Không `Debug`: có secret.
pub(crate) struct Account {
    pub api_key: SettingValue,
    pub secret_key: SettingValue,
    pub brandname: String,
    pub sandbox: bool,
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
        let sandbox =
            accounts.get(tenant, &SANDBOX).await?.is_some_and(|value| value.as_str() == "true");
        Ok(Account { api_key, secret_key, brandname, sandbox })
    }
}
