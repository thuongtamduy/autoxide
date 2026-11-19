//! Service của `shipping`: MỌI logic nằm ở đây và ở các file con (`addresses`, …). Handler REST và
//! tool MCP chỉ lấy dữ liệu vào, kiểm permission, gọi service — nên hai đường cho cùng một kết quả
//! (luật kiến trúc 8).
//!
//! Mọi hàm chạm database nhận `&TenantContext` và lọc mọi truy vấn bằng `.scoped(tenant)` (bước
//! 33). Không gọi hãng vận chuyển (mạng) trong transaction ghi.

use std::sync::Arc;

use autoxide_address::api::AdministrativeUnits;
use autoxide_kernel::error::FieldError;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{DbErr, SqlErr};

use crate::config::ShippingConfig;

/// Phụ thuộc lấy lúc khởi động.
pub(crate) struct Parts {
    pub outbox: autoxide_kernel::event::Outbox,
    pub jobs: autoxide_kernel::job::JobQueue,
    pub entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
    pub databases: autoxide_kernel::tenancy::DatabaseRegistry,
    pub secret: autoxide_kernel::secret::SecretKey,
    pub carriers: Arc<crate::registry::Registry>,
    pub ledger: Option<Arc<dyn autoxide_ledger::api::Ledger>>,
    pub reasons: autoxide_kernel::reasons::ReasonCodes,
    pub catalog: Option<Arc<dyn autoxide_catalog::api::Catalog>>,
    pub orgs: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    pub units: Option<Arc<dyn AdministrativeUnits>>,
}

/// Tính năng theo gói (bước 35).
pub(crate) const CARRIERS_FEATURE: autoxide_kernel::entitlement::Feature =
    autoxide_kernel::entitlement::Feature::new(
        "shipping.carriers",
        "Connect carrier accounts (quotes, bookings, labels and tracking from the carrier).",
    );
pub(crate) const COD_FEATURE: autoxide_kernel::entitlement::Feature =
    autoxide_kernel::entitlement::Feature::new(
        "shipping.cod",
        "Cash on delivery with reconciliation of carrier statements.",
    );
pub(crate) const PICKUP_FEATURE: autoxide_kernel::entitlement::Feature =
    autoxide_kernel::entitlement::Feature::new("shipping.pickup", "Store pickup methods.");

/// Hạn mức số tài khoản hãng của một khách.
pub(crate) const CARRIER_ACCOUNTS_LIMIT: autoxide_kernel::entitlement::Limit =
    autoxide_kernel::entitlement::Limit::count(
        "shipping.carrier_accounts",
        "Carrier accounts a tenant can connect.",
    );

pub(crate) fn features() -> Vec<autoxide_kernel::entitlement::Feature> {
    vec![CARRIERS_FEATURE, COD_FEATURE, PICKUP_FEATURE]
}

/// Mọi việc của module. Không giữ database: database là của khách, đi theo `TenantContext` của
/// từng lời gọi.
pub(crate) struct ShippingService {
    pub(crate) outbox: autoxide_kernel::event::Outbox,
    pub(crate) jobs: autoxide_kernel::job::JobQueue,
    pub(crate) entitlements: autoxide_kernel::entitlement::EntitlementsHandle,
    pub(crate) databases: autoxide_kernel::tenancy::DatabaseRegistry,
    /// Khoá gốc: mã hoá thông tin đăng nhập hãng.
    pub(crate) secret: autoxide_kernel::secret::SecretKey,
    /// Sổ hãng vận chuyển (bộ nối đăng ký sau `shipping`).
    pub(crate) carriers: Arc<crate::registry::Registry>,
    /// `dyn Ledger` khi module `ledger` đã cài: ghi nghĩa vụ COD.
    pub(crate) ledger: Option<Arc<dyn autoxide_ledger::api::Ledger>>,
    /// Danh sách lý do theo khách (`shipping.cod_resolution`).
    pub(crate) reasons: autoxide_kernel::reasons::ReasonCodes,
    /// `dyn Catalog` khi đã cài: biến thể cho ước tính công khai.
    pub(crate) catalog: Option<Arc<dyn autoxide_catalog::api::Catalog>>,
    /// `dyn OrgDirectory` khi đã cài: giờ mở cửa của cửa hàng nhận hàng.
    pub(crate) orgs: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    pub(crate) config: ShippingConfig,
    /// `dyn AdministrativeUnits` của module `address` khi đã cài: kiểm địa chỉ Việt Nam.
    pub(crate) units: Option<Arc<dyn AdministrativeUnits>>,
}

impl ShippingService {
    pub(crate) fn new(parts: Parts, config: ShippingConfig) -> Self {
        ShippingService {
            outbox: parts.outbox,
            jobs: parts.jobs,
            entitlements: parts.entitlements,
            databases: parts.databases,
            secret: parts.secret,
            carriers: parts.carriers,
            ledger: parts.ledger,
            reasons: parts.reasons,
            catalog: parts.catalog,
            orgs: parts.orgs,
            units: parts.units,
            config,
        }
    }
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên. Cắt trước để hai
/// database trả về cùng một giá trị và so bằng nhau được.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Kiểm độ dài chuỗi theo ký tự.
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

/// Cắt một thời điểm tới micro giây (như [`now`]).
pub(crate) fn micros(at: DateTime<Utc>) -> DateTime<Utc> {
    at.trunc_subsecs(6)
}

/// Lỗi vi phạm unique: thông báo của database.
pub(crate) fn unique_violation(error: &DbErr) -> Option<String> {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(message)) => Some(message),
        _ => None,
    }
}
