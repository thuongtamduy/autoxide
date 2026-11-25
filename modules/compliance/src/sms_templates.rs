//! Mẫu câu SMS mang mã một lần mà khách tự sửa (bước 70 P9, D-20261125-sms-templates): khách có brandname đăng ký
//! template với nhà mạng thì chỉ gửi được đúng câu đã đăng ký, nên câu mặc định phải đổi được theo khách.
//! Cấu hình theo khách của kernel (`PUT /api/v1/kernel/settings/{name}`), nhóm `sms_template`.

use autoxide_kernel::error::FieldError;
use autoxide_kernel::sms::{
    SMS_TEMPLATE_GROUP, check_sms_template, fill_sms_template, tenant_sms_text,
};
use autoxide_kernel::tenancy::{TenantContext, TenantSetting, TenantSettings};

use crate::service::ComplianceService;

/// Chỗ điền của cả hai mẫu.
const PLACEHOLDERS: &[&str] = &["code", "minutes"];

fn check(raw: &str) -> Result<(), FieldError> {
    check_sms_template(raw, PLACEHOLDERS)
}

/// Câu SMS mang mã xác thực số của người đã đăng nhập.
pub(crate) const PHONE_VERIFICATION: TenantSetting = TenantSetting::new(
    "compliance.sms_template.phone_verification",
    "Text of the SMS that carries the code for verifying a signed-in user's phone number, matching the \
     template registered with the carriers for the SMS brandname. Placeholders: `{code}` (required, \
     once) and `{minutes}` (minutes until the code expires). Unset: `Ma xac thuc cua ban: {code}. Het han \
     sau {minutes} phut. Khong chia se ma nay.`",
)
.group(SMS_TEMPLATE_GROUP)
.checked(check);

/// Câu SMS mang mã đăng nhập bằng số điện thoại.
pub(crate) const SIGN_IN: TenantSetting = TenantSetting::new(
    "compliance.sms_template.sign_in",
    "Text of the SMS that carries the code for signing in with a phone number, matching the template \
     registered with the carriers for the SMS brandname. Placeholders: `{code}` (required, once) and \
     `{minutes}` (minutes until the code expires). Unset: `Ma dang nhap cua ban: {code}. Het han sau \
     {minutes} phut. Khong chia se ma nay.`",
)
.group(SMS_TEMPLATE_GROUP)
.checked(check);

/// Câu mặc định khi khách chưa đặt mẫu.
const PHONE_VERIFICATION_DEFAULT: &str =
    "Ma xac thuc cua ban: {code}. Het han sau {minutes} phut. Khong chia se ma nay.";
const SIGN_IN_DEFAULT: &str =
    "Ma dang nhap cua ban: {code}. Het han sau {minutes} phut. Khong chia se ma nay.";

pub(crate) fn all() -> Vec<TenantSetting> {
    vec![PHONE_VERIFICATION, SIGN_IN]
}

impl ComplianceService {
    /// Câu SMS theo mẫu `setting` của khách, không có (hay chưa đọc được) thì câu mặc định. Không lỗi: gọi
    /// sau khi đã lưu mã mà không để lại mã không bao giờ gửi.
    pub(crate) async fn otp_text(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
        code: &str,
        minutes: i64,
    ) -> String {
        otp_text_with(self.settings.as_ref(), tenant, setting, code, minutes).await
    }
}

/// Như [`ComplianceService::otp_text`], với nguồn cấu hình `settings` (test thay bằng nguồn hỏng).
pub(crate) async fn otp_text_with(
    settings: &dyn TenantSettings,
    tenant: &TenantContext,
    setting: &TenantSetting,
    code: &str,
    minutes: i64,
) -> String {
    let minutes = minutes.to_string();
    let values = [("code", code), ("minutes", minutes.as_str())];
    if let Some(text) = tenant_sms_text(settings, tenant, setting, PLACEHOLDERS, &values).await {
        return text;
    }
    let default = if setting == &SIGN_IN { SIGN_IN_DEFAULT } else { PHONE_VERIFICATION_DEFAULT };
    fill_sms_template(default, &values).unwrap_or_else(|| default.to_owned())
}
