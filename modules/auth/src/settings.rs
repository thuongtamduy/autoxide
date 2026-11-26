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

pub(crate) fn all() -> Vec<OrgSetting> {
    vec![CUSTOMER_REGISTRATION, STAFF_REGISTRATION, SUPPORT_ACCESS]
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
