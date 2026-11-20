//! Service của module `customers`: phần dùng chung của mọi file (ngữ cảnh, giờ, che liên hệ). Logic
//! nằm ở đây và các file cùng cấp, không ở handler (luật kiến trúc 5).

use std::sync::Arc;

use autoxide_kernel::custom_fields::CustomFields;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantId};
use chrono::{DateTime, SubsecRound, Utc};

/// Mã nước mặc định khi chuẩn hoá số điện thoại nhập không có `+` (Việt Nam).
pub(crate) const DEFAULT_COUNTRY: &str = "84";

/// Loại bản ghi của hồ sơ khách trong sổ đối tượng (nhãn, ghi chú, trường tự định nghĩa — D-237).
pub(crate) const CUSTOMER: &str = "customers.customer";

pub(crate) struct CustomersService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub jobs: JobQueue,
    pub secret: SecretKey,
    /// Số đã xác thực của tài khoản (bước 37); thiếu `compliance` thì không gắn hồ sơ theo số.
    pub accounts: Option<Arc<dyn autoxide_compliance::api::AccountVerification>>,
    pub custom_fields: CustomFields,
    /// Sổ cái (nghĩa vụ điểm); chưa cài thì không ghi bút toán.
    pub ledger: Option<Arc<dyn autoxide_ledger::api::Ledger>>,
    /// Cấp voucher tặng; chưa cài thì không tặng voucher.
    pub promotions: Option<Arc<dyn autoxide_promotions::api::Promotions>>,
    /// Sổ đồng ý (tin quảng cáo); chưa cài thì không nhắc giỏ.
    pub consents: Option<Arc<dyn autoxide_compliance::api::ConsentLedger>>,
    /// Gửi tin (nhắc giỏ); chưa cài thì không gửi.
    pub notifier: Option<Arc<dyn autoxide_notifications::api::Notifier>>,
    pub config: crate::config::CustomersConfig,
    /// Giá công khai, biến thể → sản phẩm (báo yêu thích); thiếu thì không báo giảm giá.
    pub catalog: Option<Arc<dyn autoxide_catalog::api::Catalog>>,
    /// Tồn bán được (báo yêu thích có hàng lại); thiếu thì không có loại báo này.
    pub inventory: Option<Arc<dyn autoxide_inventory::api::Inventory>>,
    /// Khung giờ gửi quảng cáo (`notifications.send_windows`); thiếu thì 07:00–22:00.
    pub rules: Option<Arc<dyn autoxide_compliance::api::ComplianceRules>>,
    /// Tính năng theo khách (thẻ quà tặng, số dư).
    pub entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
}

impl CustomersService {
    /// Khoá liên hệ của số `phone` trong khách `tenant` — cùng hàm `orders` dùng cho `buyer_key`.
    pub(crate) async fn phone_key(
        &self,
        tenant: TenantId,
        phone: &PhoneNumber,
    ) -> Result<String, Error> {
        self.secret.phone_key(tenant, phone).await
    }
}

/// Giờ hiện tại, cắt tới micro giây (Postgres lưu tới micro giây; `If-Match` so đúng từng số).
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Số điện thoại đã che cho nhân viên (D-226): `+84912345412` → `09•• ••• 412`. Số ngoài Việt Nam
/// giữ mã nước: `+65•• ••• 412`.
pub(crate) fn mask_phone(e164: &str) -> String {
    // Số Việt Nam viết dạng trong nước (`09…`): giữ 2 ký tự đầu; số nước khác giữ `+` và mã nước.
    let (text, keep) = match e164.strip_prefix("+84") {
        Some(rest) => (format!("0{rest}"), 2),
        None => (e164.to_owned(), 3),
    };
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < keep + 3 {
        return "•••".to_owned();
    }
    let head: String = chars[..keep].iter().collect();
    let tail: String = chars[chars.len() - 3..].iter().collect();
    format!("{head}•• ••• {tail}")
}

/// Email đã che (D-226): `nam.nguyen@gmail.com` → `n•••@gmail.com`.
pub(crate) fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}•••@{domain}")
        }
        None => "•••".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn che_so_dien_thoai_va_email() {
        assert_eq!(mask_phone("+84912345412"), "09•• ••• 412");
        assert_eq!(mask_phone("+6591234412"), "+65•• ••• 412");
        assert_eq!(mask_email("nam.nguyen@gmail.com"), "n•••@gmail.com");
        assert_eq!(mask_email("bad"), "•••");
    }
}
