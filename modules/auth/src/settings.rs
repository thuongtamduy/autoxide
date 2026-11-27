//! Cài đặt theo cây tổ chức do `auth` khai (D-306, D-309): đọc ở nút gốc của khách qua
//! `OrgDirectory::setting` của module `organizations`.

use autoxide_kernel::tenancy::OrgSetting;
use serde_json::Value;

/// Khách cho người mua tự tạo tài khoản (`POST /register` với `realm: customer`).
pub(crate) const CUSTOMER_REGISTRATION: OrgSetting = OrgSetting::new(
    "auth.customer_registration",
    "Whether customers can create their own accounts on this tenant's shop \
     (`POST /api/v1/auth/register` with `realm: customer`). Read at the root unit.",
)
.with_default("true")
.with_schema(r#"{"type":"boolean"}"#)
.checked(boolean);

/// Khách cho người lạ tự tạo tài khoản nhân sự (`POST /register` với `realm: staff`, đăng nhập OIDC lần đầu); chưa
/// đặt thì theo biến chung `AUTOXIDE_AUTH_SELF_REGISTRATION` (mục 2 của đợt cấu hình theo khách).
pub(crate) const STAFF_REGISTRATION: OrgSetting = OrgSetting::new(
    "auth.staff_registration",
    "Whether anyone may create their own staff account in this tenant (`POST \
     /api/v1/auth/register` with `realm: staff`, or a first sign-in with a sign-in provider that \
     allows sign-up). New accounts get no role. Unset: the server's \
     AUTOXIDE_AUTH_SELF_REGISTRATION. Read at the root unit.",
)
.with_schema(r#"{"type":"boolean"}"#)
.checked(boolean);

/// Khách cho nhân sự nền tảng vào dữ liệu để hỗ trợ (bước 34 mục 13, D-211); tắt thì chỉ yêu cầu
/// của cơ quan có thẩm quyền được cấp.
pub(crate) const SUPPORT_ACCESS: OrgSetting = OrgSetting::new(
    "auth.support_access",
    "Whether platform staff may be granted time-limited access to this tenant's data for \
     support. Off: only access for a request of a competent authority can be granted. Read at \
     the root unit.",
)
.with_default("true")
.with_schema(r#"{"type":"boolean"}"#)
.checked(boolean);

/// Chính sách đăng nhập theo khách (mục 10 của đợt cấu hình theo khách, D-20261126-tenant-login-policy): chỉ siết
/// hơn cấu hình của máy chủ, không nới.
pub(crate) const PASSWORD_MIN_LENGTH: OrgSetting = OrgSetting::new(
    "auth.password_min_length",
    "Shortest password this tenant accepts, 15 to 128 characters, when a password is set or \
     changed (existing passwords keep working). It can only raise the server's minimum of 15. \
     Read at the root unit.",
)
.with_schema(r#"{"type":"integer"}"#)
.checked(password_length);

/// Thời hạn phiên đăng nhập của khách.
pub(crate) const SESSION_DAYS: OrgSetting = OrgSetting::new(
    "auth.session_days",
    "How long a sign-in lasts in this tenant, in days (1 to 365): the lifetime of each refresh \
     token, counted again at every refresh. It can only shorten the server's \
     AUTOXIDE_AUTH_REFRESH_TOKEN_DAYS, never lengthen it; sessions already open keep their \
     current token until it is refreshed. Read at the root unit.",
)
.with_schema(r#"{"type":"integer"}"#)
.checked(session_days);

/// Tên hiện cạnh tài khoản trong app xác thực của khách.
pub(crate) const TOTP_ISSUER: OrgSetting = OrgSetting::new(
    "auth.totp_issuer",
    "Name shown next to the account in authenticator apps when someone in this tenant turns on \
     two-step sign-in (1 to 100 characters, no `:`), instead of the server's \
     AUTOXIDE_AUTH_TOTP_ISSUER. Apps that already hold the account keep the old name. Read at \
     the root unit.",
)
.with_schema(r#"{"type":"string"}"#)
.checked(totp_issuer);

pub(crate) fn all() -> Vec<OrgSetting> {
    vec![
        CUSTOMER_REGISTRATION,
        STAFF_REGISTRATION,
        SUPPORT_ACCESS,
        PASSWORD_MIN_LENGTH,
        SESSION_DAYS,
        TOTP_ISSUER,
    ]
}

fn password_length(value: &Value) -> Result<Value, String> {
    // Cùng khoảng với luật của server: một giá trị dưới mức tối thiểu không có tác dụng gì nên báo sai.
    const MIN: u64 = crate::password::MIN_LENGTH as u64;
    const MAX: u64 = crate::password::MAX_LENGTH as u64;
    value
        .as_u64()
        .filter(|length| (MIN..=MAX).contains(length))
        .map(Value::from)
        .ok_or_else(|| format!("must be a whole number from {MIN} to {MAX}"))
}

fn session_days(value: &Value) -> Result<Value, String> {
    value
        .as_u64()
        .filter(|days| (1..=365).contains(days))
        .map(Value::from)
        .ok_or_else(|| "must be a whole number from 1 to 365".to_owned())
}

fn totp_issuer(value: &Value) -> Result<Value, String> {
    let name = value.as_str().map(str::trim).unwrap_or_default();
    if name.is_empty() || name.chars().count() > 100 || name.contains(':') {
        return Err("must be 1 to 100 characters, without `:`".to_owned());
    }
    Ok(Value::from(name))
}

/// Giá trị cài đặt `setting` ở nút gốc của khách (`None`: chưa đặt, hay app không có `organizations`).
pub(crate) async fn value(
    orgs: Option<&std::sync::Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    tenant: &autoxide_kernel::tenancy::TenantContext,
    setting: &OrgSetting,
) -> Result<Option<Value>, autoxide_kernel::error::Error> {
    let Some(orgs) = orgs else { return Ok(None) };
    let root = autoxide_kernel::tenancy::root_of(tenant.id());
    let found = orgs.setting(tenant, root, setting.key()).await?;
    Ok(found.and_then(|found| found.value))
}

fn boolean(value: &Value) -> Result<Value, String> {
    value.as_bool().map(Value::from).ok_or_else(|| "must be `true` or `false`".to_owned())
}

/// Giá trị bật / tắt của cài đặt `key` ở nút gốc của khách: không có `organizations` (test riêng
/// của module) hay chưa đặt thì mặc định của cài đặt.
pub(crate) async fn enabled(
    orgs: Option<&std::sync::Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    tenant: &autoxide_kernel::tenancy::TenantContext,
    setting: &OrgSetting,
) -> Result<bool, autoxide_kernel::error::Error> {
    let default = setting.default_value().and_then(|value| value.as_bool()).unwrap_or(true);
    enabled_or(orgs, tenant, setting, default).await
}

/// Như [`enabled`], chưa đặt thì `default` (cài đặt không có mặc định riêng, theo biến chung của máy chủ).
pub(crate) async fn enabled_or(
    orgs: Option<&std::sync::Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    tenant: &autoxide_kernel::tenancy::TenantContext,
    setting: &OrgSetting,
    default: bool,
) -> Result<bool, autoxide_kernel::error::Error> {
    let Some(orgs) = orgs else { return Ok(default) };
    let root = autoxide_kernel::tenancy::root_of(tenant.id());
    let found = orgs.setting(tenant, root, setting.key()).await?;
    Ok(found.and_then(|found| found.value).and_then(|value| value.as_bool()).unwrap_or(default))
}
