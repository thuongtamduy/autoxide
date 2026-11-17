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
    vec![CUSTOMER_REGISTRATION, SUPPORT_ACCESS]
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
    let Some(orgs) = orgs else { return Ok(default) };
    let root = autoxide_kernel::tenancy::root_of(tenant.id());
    let found = orgs.setting(tenant, root, setting.key()).await?;
    Ok(found.and_then(|found| found.value).and_then(|value| value.as_bool()).unwrap_or(default))
}
