//! Cấu hình theo từng khách của newsletter (D-20261125-tenant-link-templates, D-20261125-tenant-postal-address):
//! trang xác nhận và trang lựa chọn trên host của khách, địa chỉ bưu điện của người gửi.
//!
//! Địa chỉ người gửi trong thư quảng cáo, theo thứ tự: cấu hình khách `newsletter.postal_address` → hồ sơ
//! pháp lý của khách ở `compliance` (tên + địa chỉ chủ sở hữu, qua trait `LegalProfiles`) → biến chung
//! `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`. Khách B không bao giờ thấy địa chỉ của khách A.

use autoxide_compliance::api::LegalProfiles;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::tenancy::{TenantContext, TenantSetting, TenantSettings, parsed_setting};
use serde_json::Value;

use crate::campaigns::postal_address_missing;
use crate::entity::campaign;
use crate::service::NewsletterService;

pub(crate) const CONFIRM_LINK: TenantSetting = TenantSetting::link(
    "newsletter.link.confirm",
    "Page of this tenant's website that confirms a newsletter sign-up (double opt-in), such as \
     `https://shop.example/xac-nhan-dang-ky?token={token}`. Must be on this tenant's own host \
     (a verified domain of this tenant). When unset, the server's \
     AUTOXIDE_NEWSLETTER_CONFIRM_URL is used.",
    &["{token}"],
);

pub(crate) const PREFERENCES_LINK: TenantSetting = TenantSetting::link(
    "newsletter.link.preferences",
    "Page of this tenant's website where a subscriber chooses their newsletters, linked from every \
     campaign mail, such as `https://shop.example/tuy-chon-ban-tin?token={token}`. Must be on this \
     tenant's own host. When unset, the server's AUTOXIDE_NEWSLETTER_PREFERENCES_URL is used.",
    &["{token}"],
);

/// Độ dài tối đa của địa chỉ bưu điện.
const ADDRESS_MAX: usize = 300;

pub(crate) const POSTAL_ADDRESS: TenantSetting = TenantSetting::new(
    "newsletter.postal_address",
    "Postal address of the sender, shown in the footer of every campaign mail and required to send \
     advertising campaigns, such as `Example Ltd, 1 Example Street, Hanoi` (one line, at most 300 \
     characters). When unset: the owner's name and address from this tenant's legal profile \
     (module `compliance`), otherwise the server's AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS.",
)
.checked(check_address);

/// Chữ của `value` đã bỏ khoảng trắng hai đầu; `None` khi không phải chữ hay trống.
fn text(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|text| !text.is_empty())
}

fn check_address(raw: &str) -> Result<(), FieldError> {
    let raw = raw.trim();
    if !raw.is_empty() && raw.chars().count() <= ADDRESS_MAX && !raw.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(FieldError::new(
            "value",
            "newsletter.postal_address_invalid",
            "must be one line of 1 to {max} characters",
        )
        .with_param("max", ADDRESS_MAX))
    }
}

/// Phần của thư chiến dịch theo khách, tính một lần cho cả lô.
pub(crate) struct CampaignPages {
    /// Mẫu trang lựa chọn (có `{token}`).
    pub preferences: String,
    /// Địa chỉ người gửi; trống khi không có (thư không quảng cáo).
    pub address: String,
}

/// Mọi cấu hình theo khách của module.
pub(crate) fn all() -> Vec<TenantSetting> {
    vec![CONFIRM_LINK, PREFERENCES_LINK, POSTAL_ADDRESS]
}

/// Lỗi khi khách chưa có trang lựa chọn.
fn preferences_missing() -> Error {
    Error::unavailable(
        "newsletter.preferences_url_missing",
        "Campaign mails link to your website's newsletter preferences page: set the tenant setting \
         `newsletter.link.preferences` (for example \
         `https://shop.example/newsletter/preferences?token={token}`), or the server's \
         AUTOXIDE_NEWSLETTER_PREFERENCES_URL.",
    )
}

fn confirm_missing() -> Error {
    Error::unavailable(
        "newsletter.confirm_url_missing",
        "Lists with double opt-in send a confirmation link to your website: set the tenant setting \
         `newsletter.link.confirm` (for example \
         `https://shop.example/newsletter/confirm?token={token}`), or the server's \
         AUTOXIDE_NEWSLETTER_CONFIRM_URL.",
    )
}

impl NewsletterService {
    /// Trang lựa chọn của khách (có `{token}`), hay lỗi nói rõ phải đặt gì.
    pub(crate) async fn preferences_page(&self, tenant: &TenantContext) -> Result<String, Error> {
        self.links
            .link(tenant, &PREFERENCES_LINK, self.config.preferences_url.as_deref())
            .await?
            .ok_or_else(preferences_missing)
    }

    /// Trang xác nhận của khách (có `{token}`), hay lỗi nói rõ phải đặt gì.
    pub(crate) async fn confirm_page(&self, tenant: &TenantContext) -> Result<String, Error> {
        self.links
            .link(tenant, &CONFIRM_LINK, self.config.confirm_url.as_deref())
            .await?
            .ok_or_else(confirm_missing)
    }

    /// Địa chỉ người gửi của khách (xem đầu file); `None` khi không nguồn nào có.
    pub(crate) async fn postal_address(
        &self,
        tenant: &TenantContext,
    ) -> Result<Option<String>, Error> {
        resolve_postal_address(
            tenant,
            self.settings.as_deref(),
            self.legal.as_deref(),
            self.config.postal_address.as_deref(),
        )
        .await
    }

    /// Kiểm được gửi chưa: URL công khai, trang lựa chọn, địa chỉ người gửi (thư quảng cáo).
    pub(crate) async fn check_ready(
        &self,
        tenant: &TenantContext,
        row: &campaign::Model,
    ) -> Result<(), Error> {
        self.campaign_pages(tenant, row).await.map(|_| ())
    }

    /// Trang lựa chọn và địa chỉ người gửi của khách cho chiến dịch `row`, hay lỗi nói rõ còn thiếu gì. Bên gửi
    /// tính một lần cho cả lô rồi truyền vào từng thư (không đọc cấu hình theo từng người nhận).
    pub(crate) async fn campaign_pages(
        &self,
        tenant: &TenantContext,
        row: &campaign::Model,
    ) -> Result<CampaignPages, Error> {
        self.public_url.base()?;
        let preferences = self.preferences_page(tenant).await?;
        let address = self.postal_address(tenant).await?;
        if row.advertising && address.is_none() {
            return Err(postal_address_missing());
        }
        Ok(CampaignPages { preferences, address: address.unwrap_or_default() })
    }
}

/// Địa chỉ người gửi theo thứ tự ở đầu file: cấu hình khách → hồ sơ pháp lý → biến chung `fallback`.
async fn resolve_postal_address(
    tenant: &TenantContext,
    settings: Option<&dyn TenantSettings>,
    legal: Option<&dyn LegalProfiles>,
    fallback: Option<&str>,
) -> Result<Option<String>, Error> {
    if let Some(settings) = settings
        && let Some(address) = parsed_setting(settings, tenant, &POSTAL_ADDRESS, |raw| {
            check_address(raw).map(|()| raw.trim().to_owned())
        })
        .await?
    {
        return Ok(Some(address));
    }
    if let Some(legal) = legal
        && let Some(variables) = legal.template_variables(tenant, None).await?
    {
        let owner = &variables["owner"];
        if let Some(address) = text(&owner["address"]) {
            let line = match text(&owner["legal_name"]) {
                Some(name) => format!("{name}, {address}"),
                None => address.to_owned(),
            };
            return Ok(Some(line));
        }
    }
    Ok(fallback.map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::tenancy::TenantId;
    use autoxide_testkit::{FakeTenantSettings, TestApp, TestDatabase};
    use serde_json::json;

    /// Hồ sơ pháp lý giả: chỉ khách `owner` có hồ sơ.
    struct OneProfile(TenantId);

    #[async_trait::async_trait]
    impl LegalProfiles for OneProfile {
        async fn template_variables(
            &self,
            tenant: &TenantContext,
            _org: Option<autoxide_kernel::tenancy::OrgId>,
        ) -> Result<Option<Value>, Error> {
            Ok((tenant.id() == self.0).then(
                || json!({"owner": {"legal_name": "Cong ty B", "address": "2 Duong B, Da Nang"}}),
            ))
        }
    }

    #[tokio::test]
    async fn dia_chi_nguoi_gui_theo_tung_khach() {
        TestDatabase::run(async |test| {
            let app =
                TestApp::builder(test).tenant("a").tenant("b").tenant("c").build().await.unwrap();
            let (a, b, c) = (
                app.tenant("a").await.unwrap(),
                app.tenant("b").await.unwrap(),
                app.tenant("c").await.unwrap(),
            );
            let settings = FakeTenantSettings::default();
            settings.set(a.id(), "newsletter.postal_address", " Cong ty A, 1 Duong A, Ha Noi ");
            let legal = OneProfile(b.id());
            let server = Some("Operator Ltd, Hanoi");
            let resolve = async |tenant: &TenantContext| {
                resolve_postal_address(tenant, Some(&settings), Some(&legal), server).await.unwrap()
            };
            // A: cấu hình riêng; B: hồ sơ pháp lý; C: biến chung của máy chủ.
            assert_eq!(resolve(&a).await.as_deref(), Some("Cong ty A, 1 Duong A, Ha Noi"));
            assert_eq!(resolve(&b).await.as_deref(), Some("Cong ty B, 2 Duong B, Da Nang"));
            assert_eq!(resolve(&c).await.as_deref(), server);
            // Không nguồn nào: `None` (thư quảng cáo bị chặn).
            assert_eq!(
                resolve_postal_address(&c, Some(&settings), None, None).await.unwrap(),
                None
            );
        })
        .await
        .unwrap();
    }

    #[test]
    fn dia_chi_mot_dong() {
        assert!(check_address("Cong ty A, Ha Noi").is_ok());
        assert!(check_address("dong 1\ndong 2").is_err());
        assert!(check_address("  ").is_err());
        assert!(check_address(&"x".repeat(301)).is_err());
    }
}
