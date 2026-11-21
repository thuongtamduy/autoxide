//! Service của `inventory`: MỌI logic nằm ở đây và ở các file con (`stock`, `levels`…). Handler
//! REST, tool MCP, job và trait `api` chỉ lấy dữ liệu vào, kiểm permission, gọi service — nên mọi
//! đường cho cùng một kết quả (luật kiến trúc 8).
//!
//! Mọi hàm chạm database nhận `&TenantContext` và lọc mọi truy vấn bằng `.scoped(tenant)` (bước
//! 33). Tồn thuộc nút kho của cây tổ chức (bước 34): đọc kiểm `require_at`, ghi kiểm
//! `require_write_at` TRONG transaction ghi; ngoài phạm vi báo như không có (404).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_catalog::api::{Catalog, MAX_IDS, VariantSnapshot};
use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::entitlement::{EntitlementsHandle, Feature, Limit};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::reasons::ReasonCodes;
use autoxide_kernel::tenancy::{DatabaseRegistry, OrgId, ScopeExt, TenantContext, root_of};
use autoxide_organizations::api::OrgDirectory;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter};
use uuid::Uuid;

use crate::config::InventoryConfig;
use crate::entity::level;
use crate::errors;

/// Hạn mức số kho có tồn của một khách (bước 35).
pub(crate) const WAREHOUSES_LIMIT: Limit =
    Limit::count("inventory.warehouses", "Warehouses and stores that hold stock.");

/// Tính năng theo gói (bước 35).
pub(crate) const MULTI_WAREHOUSE_FEATURE: Feature =
    Feature::new("inventory.multi_warehouse", "Stock in more than one warehouse or store.");
pub(crate) const TRANSFERS_FEATURE: Feature = Feature::new(
    "inventory.transfers",
    "Stock transfers between warehouses, with goods in transit.",
);
pub(crate) const LOTS_FEATURE: Feature =
    Feature::new("inventory.lots", "Lots with expiry dates; first expired, first out.");
pub(crate) const SERIALS_FEATURE: Feature =
    Feature::new("inventory.serials", "Serial numbers tracked from receipt to sale.");
pub(crate) const PURCHASING_FEATURE: Feature = Feature::new(
    "inventory.purchasing",
    "Suppliers, purchase orders, goods receipts and supplier returns.",
);

pub(crate) fn features() -> Vec<Feature> {
    vec![
        MULTI_WAREHOUSE_FEATURE,
        TRANSFERS_FEATURE,
        LOTS_FEATURE,
        SERIALS_FEATURE,
        PURCHASING_FEATURE,
    ]
}

/// Số lượng tối đa của một dòng (một lần giữ, xuất, nhập, điều chỉnh).
pub(crate) const MAX_QUANTITY: i64 = 1_000_000;

/// Loại nút được giữ tồn (ngoài nút gốc của khách — cửa hàng đơn lẻ giữ tồn ở gốc).
const STOCK_UNIT_TYPES: [&str; 2] = ["warehouse", "store"];

/// Những gì service cần lúc khởi động.
pub(crate) struct Parts {
    pub outbox: Outbox,
    pub entitlements: EntitlementsHandle,
    pub databases: DatabaseRegistry,
    pub catalog: Arc<dyn Catalog>,
    /// `dyn OrgDirectory` của `organizations` khi đã cài: kiểm loại nút kho.
    pub orgs: Option<Arc<dyn OrgDirectory>>,
    pub reasons: ReasonCodes,
    /// `dyn Ledger` của `ledger` khi đã cài: ghi công nợ nhà cung cấp và giá trị nhập.
    pub ledger: Option<Arc<dyn autoxide_ledger::api::Ledger>>,
    /// `dyn Notifier` của `notifications` khi đã cài: báo khi có hàng lại.
    pub notifier: Option<Arc<dyn autoxide_notifications::api::Notifier>>,
    /// Chống spam cho form công khai (đăng ký báo có hàng lại).
    pub guard: Arc<autoxide_kernel::antispam::SpamGuard>,
}

/// Mọi việc của module. Không giữ database: database là của khách, đi theo `TenantContext` của
/// từng lời gọi (job và event mở ngữ cảnh qua `databases`).
pub(crate) struct InventoryService {
    pub(crate) outbox: Outbox,
    pub(crate) entitlements: EntitlementsHandle,
    pub(crate) databases: DatabaseRegistry,
    pub(crate) catalog: Arc<dyn Catalog>,
    pub(crate) orgs: Option<Arc<dyn OrgDirectory>>,
    pub(crate) reasons: ReasonCodes,
    pub(crate) ledger: Option<Arc<dyn autoxide_ledger::api::Ledger>>,
    pub(crate) notifier: Option<Arc<dyn autoxide_notifications::api::Notifier>>,
    pub(crate) guard: Arc<autoxide_kernel::antispam::SpamGuard>,
    pub(crate) config: InventoryConfig,
}

impl InventoryService {
    pub(crate) fn new(parts: Parts, config: InventoryConfig) -> Self {
        InventoryService {
            outbox: parts.outbox,
            entitlements: parts.entitlements,
            databases: parts.databases,
            catalog: parts.catalog,
            orgs: parts.orgs,
            reasons: parts.reasons,
            ledger: parts.ledger,
            notifier: parts.notifier,
            guard: parts.guard,
            config,
        }
    }

    /// Bật tính năng `feature` của gói cho khách, hay lỗi `feature_not_enabled`.
    #[allow(dead_code)]
    pub(crate) async fn require_feature(
        &self,
        tenant: &TenantContext,
        feature: &Feature,
    ) -> Result<(), Error> {
        self.entitlements.require_feature(tenant.id(), feature).await
    }

    /// Biến thể theo ID (gọi `Catalog` theo từng lô [`MAX_IDS`]); thiếu biến thể nào thì lỗi
    /// `inventory.variant_not_found`. Gọi TRƯỚC khi mở transaction (không gọi module khác trong
    /// transaction ghi).
    pub(crate) async fn variants(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, VariantSnapshot>, Error> {
        let mut unique: Vec<Uuid> = ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        let mut found = std::collections::HashMap::new();
        for chunk in unique.chunks(MAX_IDS) {
            for variant in self.catalog.variants(tenant, chunk, None).await? {
                found.insert(variant.variant_id, variant);
            }
        }
        if let Some(missing) = unique.iter().find(|id| !found.contains_key(id)) {
            return Err(errors::variant_not_found(*missing));
        }
        Ok(found)
    }

    /// Nút `org` giữ được tồn: nút gốc, hay nút loại `warehouse` / `store` (khi có
    /// `organizations`). Nút chưa có dòng tồn nào thì tính vào hạn mức `inventory.warehouses`
    /// (và cần `inventory.multi_warehouse` khi đã có kho khác).
    pub(crate) async fn ensure_warehouse(
        &self,
        tenant: &TenantContext,
        org: Uuid,
    ) -> Result<(), Error> {
        self.check_stock_unit(tenant, org).await?;
        let read = tenant.database().read();
        let has_stock = level::Entity::find()
            .filter(level::Column::OrgId.eq(org))
            .scoped(tenant)
            .count(read)
            .await?
            > 0;
        if has_stock {
            return Ok(());
        }
        let others = level::Entity::find().scoped(tenant).count(read).await? > 0;
        if others {
            self.entitlements.require_feature(tenant.id(), &MULTI_WAREHOUSE_FEATURE).await?;
        }
        let key = format!("warehouse:{org}");
        self.entitlements.reserve(tenant.id(), &WAREHOUSES_LIMIT, 1, &key).await?;
        self.entitlements.confirm(tenant.id(), &WAREHOUSES_LIMIT, &key).await
    }
}

impl InventoryService {
    /// Nút `org` giữ được tồn: nút gốc, hay nút loại `warehouse` / `store` (khi có
    /// `organizations`); không thì `inventory.not_a_warehouse`.
    pub(crate) async fn check_stock_unit(
        &self,
        tenant: &TenantContext,
        org: Uuid,
    ) -> Result<(), Error> {
        let org_id = OrgId::from_uuid(org);
        if org_id == root_of(tenant.id()) {
            return Ok(());
        }
        let Some(orgs) = &self.orgs else { return Ok(()) };
        let unit = orgs.unit(tenant, org_id).await?;
        let is_stock_unit = unit.is_some_and(|unit| {
            unit.parent_id.is_none() || STOCK_UNIT_TYPES.contains(&unit.unit_type.as_str())
        });
        if is_stock_unit { Ok(()) } else { Err(errors::not_a_warehouse(org)) }
    }
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên. Cắt trước để hai
/// database trả về cùng một giá trị.
pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}

/// Đích kiểm quyền của bản ghi thuộc nút `org`.
pub(crate) fn at(org: Uuid) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(org), None)
}

/// Đọc bản ghi thuộc nút `org`: cần `permission` phủ nút; không phủ thì báo như không có.
pub(crate) async fn readable<C: ConnectionTrait>(
    principal: &Principal,
    connection: &C,
    permission: &Permission,
    org: Uuid,
    not_found: impl FnOnce() -> Error,
) -> Result<(), Error> {
    match principal.require_at(connection, permission, at(org)).await {
        Err(error) if error.code() == "org_out_of_scope" => Err(not_found()),
        other => other,
    }
}

/// Ghi bản ghi thuộc nút `org` trong transaction: ngoài phạm vi thì `org_out_of_scope` (403),
/// nút đang khoá → `org_frozen`.
pub(crate) async fn writable<C: ConnectionTrait>(
    principal: &Principal,
    transaction: &C,
    permission: &Permission,
    org: Uuid,
) -> Result<(), Error> {
    principal.require_write_at(transaction, permission, at(org)).await
}

/// Gom lỗi trường: rỗng thì `Ok`.
pub(crate) fn fail_if_any(problems: Vec<FieldError>) -> Result<(), Error> {
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

/// Số lượng của một dòng: 1…[`MAX_QUANTITY`] (hay −[`MAX_QUANTITY`]…−1 khi `signed`, khác 0).
pub(crate) fn check_quantity(
    problems: &mut Vec<FieldError>,
    field: &str,
    quantity: i64,
    signed: bool,
) {
    let fits = if signed {
        quantity != 0 && quantity.abs() <= MAX_QUANTITY
    } else {
        (1..=MAX_QUANTITY).contains(&quantity)
    };
    if fits {
        return;
    }
    let problem = if signed {
        FieldError::new(field, "range", "must be a whole number from -{max} to {max}, not 0")
    } else {
        FieldError::new(field, "range", "must be a whole number from 1 to {max}")
    };
    problems.push(problem.with_param("max", MAX_QUANTITY));
}

/// Lỗi vi phạm unique của database.
pub(crate) fn unique_violation(error: &sea_orm::DbErr) -> bool {
    matches!(error.sql_err(), Some(sea_orm::SqlErr::UniqueConstraintViolation(_)))
}
