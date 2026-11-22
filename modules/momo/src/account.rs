//! Tài khoản MoMo của từng khách (cấu hình theo khách, nhóm `connection`) và địa chỉ API của MoMo
//! (cấu hình của người vận hành).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting};
use autoxide_kernel::error::FieldError;
use autoxide_kernel::provider::Environment;
use autoxide_kernel::tenancy::{CONNECTION_GROUP, TenantSetting};

/// Kết nối mà mọi trường bên dưới thuộc về.
pub(crate) const CONNECTION: &str = "momo.connection.account";

pub(crate) const PARTNER_CODE: TenantSetting = TenantSetting::new(
    "momo.connection.account.partner_code",
    "Partner Code of this tenant's MoMo business account (MoMo for Business portal > Integration \
     information).",
)
.group(CONNECTION_GROUP);

pub(crate) const ACCESS_KEY: TenantSetting = TenantSetting::new(
    "momo.connection.account.access_key",
    "Access Key of this tenant's MoMo business account (same page as the Partner Code).",
)
.secret()
.group(CONNECTION_GROUP);

pub(crate) const SECRET_KEY: TenantSetting = TenantSetting::new(
    "momo.connection.account.secret_key",
    "Secret Key of this tenant's MoMo business account; signs every request and checks every \
     payment notification.",
)
.secret()
.group(CONNECTION_GROUP);

pub(crate) const ENVIRONMENT: TenantSetting = TenantSetting::new(
    "momo.connection.account.environment",
    "Which MoMo environment the keys belong to: `production` (default) or `sandbox` (MoMo test \
     keys, no real money).",
)
.checked(Environment::check)
.group(CONNECTION_GROUP);

pub(crate) const RETURN_URL: TenantSetting = TenantSetting::new(
    "momo.connection.account.return_url",
    "Page of this tenant's website the buyer returns to after paying, used when a payment is \
     created without its own return URL. An `https://` address.",
)
.checked(check_return_url)
.group(CONNECTION_GROUP);

/// MoMo chuyển TRÌNH DUYỆT của người mua tới địa chỉ này (máy chủ không gọi tới nó), nên chỉ cần là
/// một URL http(s) đọc được.
pub(crate) fn valid_return_url(raw: &str) -> bool {
    reqwest::Url::parse(raw.trim())
        .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.has_host())
}

fn check_return_url(raw: &str) -> Result<(), FieldError> {
    if valid_return_url(raw) {
        Ok(())
    } else {
        Err(FieldError::new(
            "value",
            "invalid",
            "must be an address such as `https://shop.example/checkout/done`",
        ))
    }
}

pub(crate) const API_URL: Setting = Setting::new(
    "AUTOXIDE_MOMO_API_URL",
    "Address of MoMo's production payment API, without a trailing slash. Change it only to point \
     the MoMo adapter at a test double.",
)
.with_default("https://payment.momo.vn");

pub(crate) const SANDBOX_API_URL: Setting = Setting::new(
    "AUTOXIDE_MOMO_SANDBOX_API_URL",
    "Address of MoMo's sandbox payment API, used for tenants whose MoMo account is set to the \
     `sandbox` environment. Without a trailing slash.",
)
.with_default("https://test-payment.momo.vn");

/// Cấu hình của người vận hành đã đọc.
#[derive(Debug, Clone)]
pub(crate) struct MomoConfig {
    pub api_url: String,
    pub sandbox_api_url: String,
}

fn base_url(raw: &str) -> Result<String, String> {
    let url = raw.trim().trim_end_matches('/');
    let parsed = reqwest::Url::parse(url).map_err(|_| "must be an absolute address".to_owned())?;
    if !matches!(parsed.scheme(), "https" | "http") || !parsed.has_host() {
        return Err("must be an `https://` (or `http://`) address".to_owned());
    }
    Ok(url.to_owned())
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<MomoConfig, Invalid> {
    Ok(MomoConfig {
        api_url: reader.get_with(&API_URL, base_url)?,
        sandbox_api_url: reader.get_with(&SANDBOX_API_URL, base_url)?,
    })
}
