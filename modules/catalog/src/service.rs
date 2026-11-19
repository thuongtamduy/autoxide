//! Service của `catalog`: MỌI logic nằm ở đây và ở các file con (`products`, `variants`,
//! `categories`, `pricing`, `snapshots`). Handler REST và tool MCP chỉ lấy dữ liệu vào, kiểm
//! permission, gọi service — nên hai đường cho cùng một kết quả (luật kiến trúc 8).
//!
//! Mọi hàm chạm database nhận `&TenantContext` và lọc mọi truy vấn bằng `.scoped(tenant)` (bước
//! 33). Sản phẩm và bảng giá thuộc một nút của cây tổ chức (bước 34): đọc kiểm `require_at`, ghi
//! kiểm `require_write_at` TRONG transaction ghi; ngoài phạm vi báo như không có (404).

use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal, Scope};
use autoxide_kernel::entitlement::{EntitlementsHandle, Feature, Limit};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{ConnectionTrait, DbErr, SqlErr};

use crate::config::CatalogConfig;

/// Hạn mức số sản phẩm của một khách (bước 35). Sản phẩm lưu trữ vẫn tính (nó vẫn còn); xoá thì
/// trả lại.
pub(crate) const PRODUCTS_LIMIT: Limit =
    Limit::count("catalog.products", "Products a tenant can keep (archived ones included).");

/// Tính năng theo gói (bước 35).
pub(crate) const VARIANTS_FEATURE: Feature =
    Feature::new("catalog.variants", "Products with more than one variant (colour, size…).");
pub(crate) const PRICE_LISTS_FEATURE: Feature = Feature::new(
    "catalog.price_lists",
    "More than one price list: per store, per customer group, scheduled prices.",
);
pub(crate) const DIGITAL_FEATURE: Feature =
    Feature::new("catalog.digital", "Digital products delivered as download links.");
pub(crate) const BUNDLES_FEATURE: Feature =
    Feature::new("catalog.bundles", "Bundles that take stock from each component.");
pub(crate) const AI_FEATURE: Feature = Feature::new(
    "catalog.ai",
    "AI drafts of product descriptions and SEO texts (always saved as drafts).",
);

pub(crate) fn features() -> Vec<Feature> {
    vec![VARIANTS_FEATURE, PRICE_LISTS_FEATURE, DIGITAL_FEATURE, BUNDLES_FEATURE, AI_FEATURE]
}

/// Mọi việc của module. Không giữ database: database là của khách, đi theo `TenantContext` của
/// từng lời gọi.
pub(crate) struct CatalogService {
    pub(crate) outbox: Outbox,
    pub(crate) entitlements: EntitlementsHandle,
    /// `dyn Media` của module `media` khi đã cài: kiểm ảnh sản phẩm là media công khai.
    pub(crate) media: Option<Arc<dyn autoxide_media::api::Media>>,
    /// `dyn TaxEngine` của module `tax` khi đã cài: luật giá bán cho người tiêu dùng Việt Nam.
    pub(crate) tax: Option<Arc<dyn autoxide_tax::api::TaxEngine>>,
    pub(crate) config: CatalogConfig,
    /// Sổ database: database điều khiển (dữ liệu luật dùng chung) và ngữ cảnh khách cho job.
    pub(crate) databases: autoxide_kernel::tenancy::DatabaseRegistry,
    /// Hàng đợi job: ghi giá suy ra, ghi giá của lần đổi hàng loạt.
    pub(crate) jobs: autoxide_kernel::job::JobQueue,
    /// `dyn OrgDirectory` của `organizations` khi đã cài: đọc cài đặt kế thừa theo nút.
    pub(crate) organizations: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    /// Nguồn giá sau giảm tự động, do `promotions` cắm vào sau khi khởi động
    /// (`CatalogExtensions`). `OnceLock`: ghi một lần, đọc không cần khoá.
    pub(crate) sale_prices: std::sync::OnceLock<Arc<dyn crate::api::SalePriceSource>>,
}

impl crate::api::CatalogExtensions for CatalogService {
    fn set_sale_price_source(&self, source: Arc<dyn crate::api::SalePriceSource>) -> bool {
        self.sale_prices.set(source).is_ok()
    }
}

impl CatalogService {
    /// Bật tính năng `feature` của gói cho khách, hay lỗi `feature_not_enabled`.
    pub(crate) async fn require_feature(
        &self,
        tenant: &TenantContext,
        feature: &Feature,
    ) -> Result<(), Error> {
        self.entitlements.require_feature(tenant.id(), feature).await
    }

    /// Ngôn ngữ đã chuẩn hoá, hay `catalog.unsupported_locale`.
    pub(crate) fn locale(&self, locale: &str) -> Result<String, Error> {
        self.config.locale(locale).ok_or_else(|| crate::errors::unsupported_locale(locale))
    }
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên. Cắt trước để hai
/// database trả về cùng một giá trị và so bằng nhau được.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Cắt một thời điểm client gửi tới micro giây (như [`now`]).
pub(crate) fn micros(at: DateTime<Utc>) -> DateTime<Utc> {
    at.trunc_subsecs(6)
}

/// Nút của bản ghi mới khi người tạo không nêu: gốc nếu họ có `permission` cho cả khách; nút
/// duy nhất nếu mọi grant ở một nút; nhiều nút thì phải chọn (`catalog.org_required`).
pub(crate) fn default_org(
    tenant: &TenantContext,
    principal: &Principal,
    permission: &autoxide_kernel::access::Permission,
) -> Result<OrgId, Error> {
    let grants = principal.grants(permission);
    if grants.is_empty() || grants.iter().any(|grant| grant.scope == Scope::Tenant) {
        return Ok(root_of(tenant.id()));
    }
    let first = grants[0].org;
    if grants.iter().all(|grant| grant.org == first) {
        return Ok(first);
    }
    Err(Error::new(
        ErrorKind::Validation,
        "catalog.org_required",
        "You work in several organization units: give `org_id`.",
    )
    .with_field(FieldError::new(
        "org_id",
        "required",
        "you work in several organization units: choose one",
    )))
}

/// Đích kiểm quyền của bản ghi thuộc nút `org` (không có người sở hữu riêng).
pub(crate) fn at(org: uuid::Uuid) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(org), None)
}

/// Đọc bản ghi thuộc nút `org`: cần `permission` phủ nút; không phủ thì báo như không có.
pub(crate) async fn readable<C: ConnectionTrait>(
    principal: &Principal,
    connection: &C,
    permission: &autoxide_kernel::access::Permission,
    org: uuid::Uuid,
    not_found: impl FnOnce() -> Error,
) -> Result<(), Error> {
    match principal.require_at(connection, permission, at(org)).await {
        Err(error) if error.code() == "org_out_of_scope" => Err(not_found()),
        other => other,
    }
}

/// Ghi bản ghi thuộc nút `org` trong transaction: không đọc được thì 404 (không lộ bản ghi tồn
/// tại), đọc được mà không ghi được thì 403; nút đang khoá → `org_frozen`.
pub(crate) async fn writable<C: ConnectionTrait>(
    principal: &Principal,
    transaction: &C,
    read: &autoxide_kernel::access::Permission,
    write: &autoxide_kernel::access::Permission,
    org: uuid::Uuid,
    not_found: impl FnOnce() -> Error,
) -> Result<(), Error> {
    let target = at(org);
    if principal.require_at(transaction, read, target).await.is_err()
        && principal.require_at(transaction, write, target).await.is_err()
    {
        return Err(not_found());
    }
    principal.require_write_at(transaction, write, target).await
}

/// Lỗi vi phạm unique: thông báo của database (SQLite nêu tên cột, Postgres nêu tên index).
pub(crate) fn unique_violation(error: &DbErr) -> Option<String> {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(message)) => Some(message),
        _ => None,
    }
}

/// Gom lỗi trường: rỗng thì `Ok`.
pub(crate) fn fail_if_any(problems: Vec<FieldError>) -> Result<(), Error> {
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

/// Kiểm độ dài chuỗi theo ký tự (sau khi bỏ khoảng trắng hai đầu khi `trim`).
pub(crate) fn check_length(
    problems: &mut Vec<FieldError>,
    field: &str,
    value: &str,
    min: usize,
    max: usize,
) {
    let count = value.chars().count();
    if count < min || count > max {
        problems.push(
            FieldError::new(field, "length", "must be {min} to {max} characters long")
                .with_param("min", min)
                .with_param("max", max),
        );
    }
}

/// Ô JSON `{locale: chữ}` lưu trong cột `text`.
pub(crate) fn texts_json(texts: &std::collections::BTreeMap<String, String>) -> String {
    serde_json::to_string(texts).unwrap_or_else(|_| "{}".to_owned())
}

/// Đọc ô JSON `{locale: chữ}`; hỏng thì rỗng.
pub(crate) fn texts_of(raw: &str) -> std::collections::BTreeMap<String, String> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// Chữ theo `locale`, không có thì theo `fallback`, không có nữa thì chữ đầu tiên.
pub(crate) fn pick_text(
    texts: &std::collections::BTreeMap<String, String>,
    locale: Option<&str>,
    fallback: &str,
) -> String {
    locale
        .and_then(|locale| texts.get(locale))
        .or_else(|| texts.get(fallback))
        .or_else(|| texts.values().next())
        .cloned()
        .unwrap_or_default()
}

/// Bộ đếm thật cho job đối soát hạn mức: số sản phẩm của khách (kể cả đã lưu trữ).
pub(crate) struct ProductCounter;

#[async_trait::async_trait]
impl autoxide_kernel::entitlement::UsageCounter for ProductCounter {
    async fn count(&self, tenant: &TenantContext) -> Result<i64, Error> {
        use autoxide_kernel::tenancy::ScopeExt;
        use sea_orm::{EntityTrait, PaginatorTrait};
        let found = crate::entity::product::Entity::find()
            .scoped(tenant)
            .count(tenant.database().read())
            .await?;
        Ok(i64::try_from(found).unwrap_or(i64::MAX))
    }
}
