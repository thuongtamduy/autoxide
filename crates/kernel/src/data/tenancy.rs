//! Nền đa khách (bước 33, D-185, D-253): một bản cài phục vụ nhiều **khách hàng** (tenant) mà dữ
//! liệu không bao giờ lẫn nhau.
//!
//! - [`TenantId`]: ID của khách. [`DEFAULT_TENANT`] là khách mặc định — cũng là khách nền tảng,
//!   nơi tài khoản của nhà vận hành sống; dữ liệu của bản cài cũ thuộc về nó.
//! - [`TenantContext`]: khách của một request / job / event, kèm database chứa dữ liệu của khách.
//!   Mọi hàm service chạm dữ liệu cấp khách hàng nhận nó và lấy database qua
//!   [`TenantContext::database`] — quên khách thì không có database để truy vấn (D-253 mục 1).
//! - [`DatabaseRegistry`]: sổ database — database điều khiển (`DATABASE_URL`) và các placement
//!   (vị trí dữ liệu: chung `shared` hay riêng `dedicated`), mở lười, có giới hạn.
//! - Chế độ ([`TenancyMode`], `AUTOXIDE_TENANCY_MODE`): `single` (mặc định — một khách, không truy
//!   vấn gì thêm mỗi request) hay `multi`.
//! - Tên miền riêng của khách ([`TenantDomains`], `domains.rs`): xác minh bằng bản ghi TXT, request
//!   tới tên miền đã xác minh vào thẳng khách (D-190).
//! - Token trong link của thư mang khách ([`link_token`], [`resolve_link_token`],
//!   [`OptionalTenant`]): bấm link ở tên miền chính vẫn tới đúng khách (D-281).
//!
//! Bảng mặt điều khiển (`kernel_tenants`, `kernel_placements`, `kernel_tenant_domains`) do
//! module nội bộ `kernel` tạo (`schema.rs`); chúng chỉ có dữ liệu ở database điều khiển.

pub(crate) mod admin;
mod config;
pub(crate) mod domains;
mod link_token;
pub mod migration;
mod placements;
mod registry;
pub(crate) mod request;
pub(crate) mod schema;
mod scope;
pub(crate) mod settings;

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::database::Database;
use crate::error::Error;

pub use admin::{
    CurrentTenant, NewTenant, PLACEMENTS_READ, Placement, TENANTS_MANAGE, TENANTS_READ, Tenant,
    TenantAdmin, TenantChanges, TenantFilter, TenantSeed, require_platform,
};
pub use config::{TenancyConfig, WorkerPlacements};
pub use domains::{
    CHALLENGE_PREFIX, CHALLENGE_VALUE_PREFIX, DOMAINS_MANAGE, DnsTxtLookup, DomainRecord,
    TenantDomain, TenantDomains, normalize_domain,
};
pub use link_token::{OptionalTenant, link_token, resolve_link_token, split_link_token};
pub use placements::{NewPlacement, PlacementCheck, PlacementMigrations, Placements, SchemaState};
pub use registry::{DatabaseRegistry, PLACEMENT_READ_URL, PLACEMENT_URL};
pub use request::{TENANT_HEADER, TenantHint};
pub use scope::{ScopeExt, TenantScoped};
pub use settings::{PUBLIC_URL, SETTINGS_MANAGE, SettingValue, TenantSetting, TenantSettings};

pub(crate) use config::read_setting;

/// ID của một khách hàng.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(Uuid);

impl TenantId {
    /// Khách có ID `id`.
    pub const fn from_uuid(id: Uuid) -> Self {
        TenantId(id)
    }

    /// UUID của khách (để ghi vào cột `tenant_id`).
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<TenantId> for Uuid {
    fn from(id: TenantId) -> Self {
        id.0
    }
}

/// ID của một nút trong cây tổ chức của khách. Khai sẵn cho bước 34; chưa dùng.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OrgId(Uuid);

impl OrgId {
    pub const fn from_uuid(id: Uuid) -> Self {
        OrgId(id)
    }

    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

/// ID của một placement (vị trí dữ liệu: một database).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlacementId(Uuid);

impl PlacementId {
    pub const fn from_uuid(id: Uuid) -> Self {
        PlacementId(id)
    }

    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for PlacementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Khách mặc định — cũng là khách nền tảng. ID cố định đúng dạng UUIDv7: migration của module gán
/// dữ liệu cũ cho nó mà không phải đọc bảng của `kernel` (ngoại lệ có chủ đích, D-253 mục 3).
pub const DEFAULT_TENANT: TenantId =
    TenantId(Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0001));

/// "Không khách nào": chỉ cho job của cả database (lịch dọn dẹp) trong bảng `<m>_jobs`. Không dùng
/// `NULL` vì unique index coi các `NULL` là khác nhau (D-253 mục 4).
pub const NO_TENANT: TenantId = TenantId(Uuid::nil());

/// Placement `main`: database điều khiển (`DATABASE_URL`). Khách mặc định nằm ở đây.
pub const MAIN_PLACEMENT: PlacementId =
    PlacementId(Uuid::from_u128(0x0000_0000_0000_7000_8000_0000_0000_0002));

/// `true` từ khi mọi module đã chuyển sang theo khách (bước 33 B14, D-253 mục 6): bật được chế độ
/// `multi` bằng `AUTOXIDE_TENANCY_MODE`.
pub const MULTI_TENANT_READY: bool = true;

/// Chế độ của bản cài (`AUTOXIDE_TENANCY_MODE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenancyMode {
    /// Một khách (khách mặc định); không đọc tên miền hay header, không truy vấn gì thêm.
    #[default]
    Single,
    /// Nhiều khách: khách suy từ credential, header, tên miền.
    Multi,
}

/// Trạng thái của một khách.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStatus {
    /// Đang cấp phát (tạo database, gieo dữ liệu ban đầu); chưa phục vụ.
    Provisioning,
    Active,
    /// Tạm khoá: mọi route của khách trả `tenant_suspended`.
    Suspended,
    /// Bảo trì: mọi route của khách trả `tenant_maintenance`.
    Maintenance,
    /// Đang dời sang database khác: như bảo trì.
    Moving,
}

impl TenantStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TenantStatus::Provisioning => "provisioning",
            TenantStatus::Active => "active",
            TenantStatus::Suspended => "suspended",
            TenantStatus::Maintenance => "maintenance",
            TenantStatus::Moving => "moving",
        }
    }

    /// Trạng thái lưu trong cột `status`.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "provisioning" => Some(TenantStatus::Provisioning),
            "active" => Some(TenantStatus::Active),
            "suspended" => Some(TenantStatus::Suspended),
            "maintenance" => Some(TenantStatus::Maintenance),
            "moving" => Some(TenantStatus::Moving),
            _ => None,
        }
    }
}

/// Loại placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementKind {
    /// Nhiều khách chung một database (phân biệt bằng `tenant_id`).
    Shared,
    /// Database riêng của một khách.
    Dedicated,
}

impl PlacementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PlacementKind::Shared => "shared",
            PlacementKind::Dedicated => "dedicated",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "shared" => Some(PlacementKind::Shared),
            "dedicated" => Some(PlacementKind::Dedicated),
            _ => None,
        }
    }
}

/// Khách của một request, job hay event, kèm database chứa dữ liệu của khách. Clone rẻ (bên trong
/// là `Arc`); hai ngữ cảnh bằng nhau khi cùng khách.
#[derive(Clone)]
pub struct TenantContext(Arc<Inner>);

#[derive(Clone)]
struct Inner {
    id: TenantId,
    slug: String,
    name: String,
    status: TenantStatus,
    placement: PlacementId,
    database: Database,
    /// Placement đã áp dụng đủ migration (`schema_state = ready`); chưa thì khách nhận
    /// `tenant_maintenance`.
    schema_ready: bool,
}

impl TenantContext {
    /// Ngữ cảnh của khách `id` trên `database` (sổ database và testkit dựng; module không tự dựng).
    pub fn new(
        id: TenantId,
        slug: impl Into<String>,
        name: impl Into<String>,
        status: TenantStatus,
        placement: PlacementId,
        database: Database,
    ) -> Self {
        TenantContext(Arc::new(Inner {
            id,
            slug: slug.into(),
            name: name.into(),
            status,
            placement,
            database,
            schema_ready: true,
        }))
    }

    /// Cùng khách, đánh dấu placement chưa áp dụng đủ migration (sổ database dựng khi
    /// `schema_state` của placement khác `ready`).
    #[must_use]
    pub fn with_schema_ready(self, ready: bool) -> Self {
        let inner = Arc::unwrap_or_clone(self.0);
        TenantContext(Arc::new(Inner { schema_ready: ready, ..inner }))
    }

    /// Placement của khách đã áp dụng đủ migration.
    pub fn schema_ready(&self) -> bool {
        self.0.schema_ready
    }

    /// Khách mặc định trên `database` — chế độ `single`, và đường chuyển tiếp cho code chưa nhận
    /// ngữ cảnh khách (D-253).
    pub fn default_on(database: Database) -> Self {
        Self::new(
            DEFAULT_TENANT,
            "default",
            "Default",
            TenantStatus::Active,
            MAIN_PLACEMENT,
            database,
        )
    }

    /// Đường chuyển tiếp (bước 33 A4): code chưa nhận ngữ cảnh khách (module chưa chuyển ở phần B)
    /// dùng khách mặc định trên database nó đang giữ. Bị `#[deprecated]` khi phần B xong (không còn
    /// ai gọi), xoá ở đầu bước 34 (D-253 mục 7).
    #[doc(hidden)]
    #[deprecated(
        note = "dùng `TenantContext` của request, job hay event; cần khách mặc định thì `default_on`"
    )]
    pub fn legacy_default(database: &Database) -> Self {
        Self::default_on(database.clone())
    }

    pub fn id(&self) -> TenantId {
        self.0.id
    }

    /// Tên rút gọn (chữ thường, số, `-`), duy nhất trên nền tảng.
    pub fn slug(&self) -> &str {
        &self.0.slug
    }

    pub fn name(&self) -> &str {
        &self.0.name
    }

    pub fn status(&self) -> TenantStatus {
        self.0.status
    }

    pub fn placement(&self) -> PlacementId {
        self.0.placement
    }

    /// Database chứa dữ liệu của khách.
    pub fn database(&self) -> &Database {
        &self.0.database
    }

    /// Khách nền tảng (khách mặc định): nơi nhân sự của nhà vận hành sống.
    pub fn is_platform(&self) -> bool {
        self.0.id == DEFAULT_TENANT
    }

    /// Tiền tố khoá file lưu trữ của khách: rỗng cho khách mặc định (file cũ không phải dời),
    /// `tenants/<id>/` cho khách khác.
    pub fn storage_prefix(&self) -> String {
        if self.is_platform() { String::new() } else { format!("tenants/{}/", self.0.id) }
    }
}

impl PartialEq for TenantContext {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for TenantContext {}

impl fmt::Debug for TenantContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TenantContext")
            .field("id", &self.0.id)
            .field("slug", &self.0.slug)
            .field("status", &self.0.status)
            .field("placement", &self.0.placement)
            .finish_non_exhaustive()
    }
}

/// Lỗi chung của phần đa khách (mã ổn định, bản dịch ở `crates/kernel/locales/vi.json`).
pub mod errors {
    use crate::error::Error;

    /// Không xác định được khách của request.
    pub fn tenant_required() -> Error {
        Error::bad_request(
            "tenant_required",
            "No customer (tenant) could be told for this request. Send it to the tenant's domain, \
             add the header `Autoxide-Tenant: <slug>`, or sign in.",
        )
    }

    pub fn tenant_not_found(tenant: &str) -> Error {
        Error::not_found("tenant_not_found", "There is no tenant `{tenant}`.")
            .with_param("tenant", tenant)
    }

    pub fn tenant_suspended() -> Error {
        Error::forbidden(
            "tenant_suspended",
            "This tenant is suspended. Contact the service provider.",
        )
    }

    pub fn tenant_maintenance(seconds: u64) -> Error {
        Error::unavailable(
            "tenant_maintenance",
            "The data of this tenant is under maintenance. Retry in {seconds} seconds.",
        )
        .with_param("seconds", seconds)
        .with_retry_after(seconds)
    }

    pub fn tenant_mismatch() -> Error {
        Error::forbidden(
            "tenant_mismatch",
            "The credentials belong to another tenant than the one this request is for.",
        )
    }

    pub fn tenant_platform_only() -> Error {
        Error::forbidden("tenant_platform_only", "Only administrators of the platform can do this.")
    }

    pub fn tenant_single_mode() -> Error {
        Error::conflict(
            "tenant_single_mode",
            "This installation runs in single-tenant mode; set AUTOXIDE_TENANCY_MODE=multi to add \
             tenants.",
        )
    }

    pub fn tenant_placement_taken(placement: &str) -> Error {
        Error::conflict(
            "tenant_placement_taken",
            "There is already a placement named `{placement}`.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_placement_behind(placement: &str) -> Error {
        Error::conflict(
            "tenant_placement_behind",
            "The placement `{placement}` has migrations to apply: run `autoxide migrate up --all` \
             first.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_slug_taken(slug: &str) -> Error {
        Error::conflict("tenant_slug_taken", "Another tenant already uses the slug `{slug}`.")
            .with_param("slug", slug)
    }

    pub fn tenant_setting_unknown(name: &str) -> Error {
        Error::new(
            crate::error::ErrorKind::Validation,
            "tenant_setting_unknown",
            "No module takes the setting `{name}`. List them with `GET /api/v1/kernel/settings`.",
        )
        .with_param("name", name)
    }

    pub fn tenant_placement_not_found(placement: &str) -> Error {
        Error::not_found("tenant_placement_not_found", "There is no placement `{placement}`.")
            .with_param("placement", placement)
    }

    pub fn tenant_domain_invalid(domain: &str) -> Error {
        // Giá trị do client gửi: chỉ nhắc lại phần đầu.
        let shown: String = domain.chars().take(100).collect();
        Error::new(
            crate::error::ErrorKind::Validation,
            "tenant_domain_invalid",
            "`{domain}` is not a domain that can be used: send a name such as `shop.example` \
             (punycode for international names), not an IP address, `localhost` or the \
             platform's own domain.",
        )
        .with_param("domain", shown)
    }

    pub fn tenant_domain_taken(domain: &str) -> Error {
        Error::conflict("tenant_domain_taken", "The domain `{domain}` belongs to another tenant.")
            .with_param("domain", domain)
    }

    pub fn tenant_domain_not_found() -> Error {
        Error::not_found(
            "tenant_domain_not_found",
            "This domain is not one of the current tenant's domains.",
        )
    }

    pub fn tenant_domain_dns_failed(error: &str) -> Error {
        Error::unavailable("tenant_domain_dns_failed", "The DNS lookup failed: {error}")
            .with_param("error", error)
    }
}

/// Khách mặc định trên một database không nối tới đâu — cho test đơn vị của kernel không truy vấn.
#[cfg(test)]
pub(crate) fn test_tenant() -> TenantContext {
    TenantContext::default_on(Database::detached())
}

/// Kiểm `tenant` có phục vụ được không: tạm khoá → `tenant_suspended`; bảo trì / đang dời / đang
/// cấp phát, hay placement của khách chưa áp dụng đủ migration → `tenant_maintenance`.
pub fn ensure_serving(tenant: &TenantContext) -> Result<(), Error> {
    if !tenant.schema_ready() && tenant.status() != TenantStatus::Suspended {
        return Err(errors::tenant_maintenance(30));
    }
    match tenant.status() {
        TenantStatus::Active => Ok(()),
        TenantStatus::Suspended => Err(errors::tenant_suspended()),
        TenantStatus::Maintenance | TenantStatus::Moving | TenantStatus::Provisioning => {
            Err(errors::tenant_maintenance(30))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_co_dinh_dung_dang_uuid_v7() {
        assert_eq!(DEFAULT_TENANT.to_string(), "00000000-0000-7000-8000-000000000001");
        assert_eq!(DEFAULT_TENANT.as_uuid().get_version_num(), 7);
        assert_eq!(MAIN_PLACEMENT.as_uuid().get_version_num(), 7);
        assert!(NO_TENANT.as_uuid().is_nil());
    }

    #[test]
    fn trang_thai_doc_ghi_du() {
        for status in [
            TenantStatus::Provisioning,
            TenantStatus::Active,
            TenantStatus::Suspended,
            TenantStatus::Maintenance,
            TenantStatus::Moving,
        ] {
            assert_eq!(TenantStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(TenantStatus::parse("gone"), None);
        for kind in [PlacementKind::Shared, PlacementKind::Dedicated] {
            assert_eq!(PlacementKind::parse(kind.as_str()), Some(kind));
        }
    }
}
