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
//! - Dời khách giữa các placement ([`TenantMoves`], `moves.rs`): `shared` ↔ `dedicated`, chạy lại
//!   được khi hỏng giữa chừng.
//! - Token trong link của thư mang khách ([`link_token`], [`resolve_link_token`],
//!   [`OptionalTenant`]): bấm link ở tên miền chính vẫn tới đúng khách (D-281).
//!
//! Bảng mặt điều khiển (`kernel_tenants`, `kernel_placements`, `kernel_tenant_domains`) do
//! module nội bộ `kernel` tạo (`schema.rs`); chúng chỉ có dữ liệu ở database điều khiển.

pub(crate) mod admin;
mod config;
mod current;
pub mod distribution;
pub(crate) mod domains;
pub(crate) mod erasure;
pub(crate) mod exports;
mod link_token;
pub mod migration;
pub(crate) mod migration_runs;
pub(crate) mod moves;
pub(crate) mod org_settings;
pub(crate) mod org_tree;
mod placements;
mod registry;
pub mod relocate;
pub(crate) mod request;
pub(crate) mod residency;
pub mod sandbox;
pub(crate) mod schema;
mod scope;
pub(crate) mod settings;
mod site;

use crate::time::Timestamp;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::database::Database;
use crate::error::Error;

pub use admin::{
    NewTenant, PLACEMENTS_READ, Placement, PlacementStorage, TENANTS_MANAGE, TENANTS_READ, Tenant,
    TenantAdmin, TenantChanges, TenantExpanded, TenantFilter, TenantSeed, require_platform,
};
pub use config::{TenancyConfig, WorkerPlacements};
pub use current::CurrentTenant;
pub use domains::{
    CHALLENGE_PREFIX, CHALLENGE_VALUE_PREFIX, DOMAINS_MANAGE, DnsTxtLookup, DomainRecord,
    TenantDomain, TenantDomains, normalize_domain,
};
pub use erasure::{TENANTS_DELETE, TenantErasure};
pub use exports::{DATA_EXPORT, TenantExport, TenantExportLink, TenantExports};
pub use link_token::{OptionalTenant, link_token, resolve_link_token, split_link_token};
pub use moves::{
    TENANTS_MOVE, TenantMove, TenantMoveBlocker, TenantMovePlan, TenantMoves, TenantTableRows,
};
pub use org_settings::{OrgSetting, OrgSettingCatalog, OrgSettingCheck};
pub use org_tree::{OrgTree, nearest, root_immutable, root_of};
pub use placements::{NewPlacement, PlacementCheck, PlacementMigrations, Placements, SchemaState};
pub use registry::{
    ChildTenant, DatabaseRegistry, FileStore, PLACEMENT_READ_URL, PLACEMENT_STORAGE, PLACEMENT_URL,
};
pub use relocate::{
    Merge, RELOCATIONS_REQUEST, RelocateTable, Relocation, RelocationBlocker, RelocationConflict,
    RelocationContext, RelocationInput, RelocationKind, RelocationPlan, RelocationSwitch,
    RelocationTable, RelocationWarning, SealedColumn, Split, SwitchHook, TENANTS_RELOCATE,
    TablePolicy, TableReport, TenantRelocation, TenantRelocations,
};
pub use request::{TENANT_HEADER, TenantHint};
pub use residency::{Residency, parse_country};
pub use sandbox::{
    NewSandbox, SANDBOXES_LIMIT, SANDBOXES_MANAGE, SANDBOXES_READ, Sandbox, TenantKind,
    TenantSandboxes,
};
pub use scope::{ScopeExt, TenantScoped};
pub use settings::{
    BUSINESS_MODEL, BUSINESS_MODELS, CONNECTION_GROUP, PUBLIC_URL, SETTINGS_MANAGE, SettingCheck,
    SettingValue, SiteLinks, TenantSetting, TenantSettings, parsed_setting,
};
pub use site::{SITE_PARAM, Site, SiteQuery, site_not_found};

pub(crate) use config::read_setting;

/// Chỗ module `kernel` nhận thứ của mọi module khi chúng đã khởi động (`Modules::start_in` điền):
/// bộ gieo dữ liệu cho khách mới, bộ tra DNS, cây tổ chức, thứ việc dời khách cần.
pub(crate) struct Slots {
    pub(crate) seeds: admin::Seeds,
    pub(crate) dns: domains::DnsSlot,
    pub(crate) orgs: domains::OrgSlot,
    pub(crate) moves: moves::MoveSetup,
}

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

/// ID của một nút trong cây tổ chức của khách (bước 34). Tên miền gắn được với một nút (đối chiếu
/// thiết kế đợt 2).
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

impl fmt::Display for OrgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Cây tổ chức của khách, nhìn từ kernel: chỉ hỏi "nút này có thuộc khách không". Module giữ cây
/// tổ chức (bước 34) công bố `context.provide::<dyn OrgNodes>(…)` trong `Module::start`; chưa
/// module nào công bố thì kernel coi như khách không có nút nào (vd tên miền chỉ gắn được `null`).
///
/// Tên khác `OrgDirectory` của module `organizations` (bước 34, hợp đồng đầy đủ của cây tổ chức
/// cho module khác): đây là phần hẹp kernel cần, kernel không phụ thuộc module nào.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait OrgNodes: Send + Sync {
    /// `org` là một nút (còn dùng được) trong cây tổ chức của khách `tenant`.
    async fn has_node(&self, tenant: &TenantContext, org: OrgId) -> Result<bool, Error>;
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
    /// Đã xoá hẳn (dữ liệu đã xoá, chỉ còn dòng trong sổ khách): request nhận `tenant_not_found`.
    Deleted,
    /// Đã gộp vào khách khác (bước 38): dữ liệu đã chuyển sang khách nhận; request nhận
    /// `tenant_not_found`.
    Merged,
}

impl TenantStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TenantStatus::Provisioning => "provisioning",
            TenantStatus::Active => "active",
            TenantStatus::Suspended => "suspended",
            TenantStatus::Maintenance => "maintenance",
            TenantStatus::Moving => "moving",
            TenantStatus::Deleted => "deleted",
            TenantStatus::Merged => "merged",
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
            "deleted" => Some(TenantStatus::Deleted),
            "merged" => Some(TenantStatus::Merged),
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
    /// Nơi lưu file riêng của placement (`autoxide placement storage`); `None`: nơi lưu chung.
    storage: Option<Arc<dyn crate::storage::Storage>>,
    /// Chỉ đọc (`kernel_tenants.read_only`): lệnh ghi bị chặn ở chỗ giải khách của request.
    read_only: bool,
    /// Sandbox (D-232): khách gốc và hạn; `None` với khách thật.
    sandbox: Option<(TenantId, Option<Timestamp>)>,
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
            storage: None,
            read_only: false,
            sandbox: None,
        }))
    }

    /// Cùng khách, file lưu ở `storage` (nơi lưu riêng của placement của nó) thay cho nơi lưu chung.
    /// Sổ database dựng; test dựng để thử placement có nơi lưu riêng.
    #[must_use]
    pub fn with_storage(self, storage: Arc<dyn crate::storage::Storage>) -> Self {
        let inner = Arc::unwrap_or_clone(self.0);
        TenantContext(Arc::new(Inner { storage: Some(storage), ..inner }))
    }

    /// Nơi lưu file riêng của placement của khách; `None`: khách dùng nơi lưu chung.
    /// [`TenantStorage::new`](crate::storage::TenantStorage::new) tự chọn.
    pub fn own_storage(&self) -> Option<&Arc<dyn crate::storage::Storage>> {
        self.0.storage.as_ref()
    }

    /// Placement có nơi lưu riêng mà khách dùng; `None`: khách dùng nơi lưu chung. Job dọn file
    /// mồ côi so với [`FileStore::placement`] để chỉ xét khách của nơi lưu đang quét.
    pub fn storage_placement(&self) -> Option<PlacementId> {
        self.0.storage.as_ref().map(|_| self.0.placement)
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

    /// Cùng khách, đánh dấu chỉ đọc (sổ database dựng từ `kernel_tenants.read_only`; test dựng).
    #[must_use]
    pub fn with_read_only(self, read_only: bool) -> Self {
        let inner = Arc::unwrap_or_clone(self.0);
        TenantContext(Arc::new(Inner { read_only, ..inner }))
    }

    /// Khách chỉ đọc: đọc được, lệnh ghi qua REST / MCP bị chặn (`tenant_read_only`). Job và event
    /// của khách vẫn chạy (việc đã nhận trước đó, dọn dẹp theo luật).
    pub fn read_only(&self) -> bool {
        self.0.read_only
    }

    /// Cùng khách, là sandbox của khách `of`, hết hạn lúc `expires_at` (sổ database dựng từ
    /// `kernel_tenants`; test dựng).
    #[must_use]
    pub fn with_sandbox_of(self, of: TenantId, expires_at: Option<Timestamp>) -> Self {
        let inner = Arc::unwrap_or_clone(self.0);
        TenantContext(Arc::new(Inner { sandbox: Some((of, expires_at)), ..inner }))
    }

    /// Khách thử nghiệm (sandbox, D-232): đối tác luôn là bộ giả, mail và webhook không ra ngoài.
    pub fn is_sandbox(&self) -> bool {
        self.0.sandbox.is_some()
    }

    /// Khách gốc của sandbox; `None` với khách thật.
    pub fn sandbox_of(&self) -> Option<TenantId> {
        self.0.sandbox.map(|(of, _)| of)
    }

    /// Hạn của sandbox; `None` với khách thật.
    pub fn sandbox_expires_at(&self) -> Option<Timestamp> {
        self.0.sandbox.and_then(|(_, at)| at)
    }

    /// Loại khách.
    pub fn kind(&self) -> TenantKind {
        if self.is_sandbox() { TenantKind::Sandbox } else { TenantKind::Production }
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
            .field("read_only", &self.0.read_only)
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

    pub fn tenant_read_only() -> Error {
        Error::forbidden(
            "tenant_read_only",
            "This tenant is read-only for now: its data can be read and exported, but not \
             changed. Contact the service provider.",
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

    pub fn tenant_residency_violation(country: &str) -> Error {
        Error::conflict(
            "tenant_residency_violation",
            "The tenant must keep its data in {country}; this placement does not meet that (its \
             country is unknown or different). Set the placement's country with `autoxide \
             placement country`, or choose a placement in {country}.",
        )
        .with_param("country", country)
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

    pub fn tenant_placement_storage_in_use(placement: &str, tenants: u64) -> Error {
        Error::conflict(
            "tenant_placement_storage_in_use",
            "{tenants} tenant(s) keep their data on the placement `{placement}`; changing its file \
             storage would hide their files. Move them to another placement first, or pass \
             `--force` when the new storage already holds the same files (e.g. a new access key).",
        )
        .with_param("placement", placement)
        .with_param("tenants", tenants)
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

    pub fn tenant_domain_revoked() -> Error {
        Error::conflict(
            "tenant_domain_revoked",
            "This domain was removed from the tenant and cannot be changed. Add it again with \
             `POST /api/v1/kernel/domains`.",
        )
    }

    pub fn tenant_domain_dns_failed(error: &str) -> Error {
        Error::unavailable("tenant_domain_dns_failed", "The DNS lookup failed: {error}")
            .with_param("error", error)
    }

    pub fn tenant_moving() -> Error {
        Error::conflict(
            "tenant_moving",
            "The tenant is being moved to another placement. Wait until the move is done, or \
             abort it with `autoxide tenant move <tenant> --abort`.",
        )
    }

    pub fn tenant_move_platform() -> Error {
        Error::conflict(
            "tenant_move_platform",
            "The platform tenant always keeps its data on the placement `main`; it cannot be moved.",
        )
    }

    pub fn tenant_move_same_placement(placement: &str) -> Error {
        Error::conflict(
            "tenant_move_same_placement",
            "The tenant already keeps its data on the placement `{placement}`.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_move_not_active(status: &str) -> Error {
        Error::conflict(
            "tenant_move_not_active",
            "Only an active tenant can be moved; this one is `{status}`.",
        )
        .with_param("status", status)
    }

    pub fn tenant_move_running(placement: &str) -> Error {
        Error::conflict(
            "tenant_move_running",
            "A move of this tenant to the placement `{placement}` is running now. Follow it with \
             the move status (`GET /api/v1/kernel/tenants/<tenant>/moves/<move>`) and try again when it is done.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_move_unfinished(placement: &str) -> Error {
        Error::conflict(
            "tenant_move_unfinished",
            "A move of this tenant to the placement `{placement}` stopped before the end. Resume \
             it by moving the tenant to `{placement}` again, or abort it with \
             `autoxide tenant move <tenant> --abort`.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_move_not_abortable() -> Error {
        Error::conflict(
            "tenant_move_not_abortable",
            "The tenant already uses its new placement; the move only has to clean the old one. \
             Resume it instead of aborting: move the tenant to the same placement again.",
        )
    }

    pub fn tenant_move_schema_differs(placement: &str, pending: usize) -> Error {
        Error::conflict(
            "tenant_move_schema_differs",
            "The placement `{placement}` has {pending} migration(s) not applied: run \
             `autoxide migrate up --all` so both databases have the same schema.",
        )
        .with_param("placement", placement)
        .with_param("pending", pending)
    }

    pub fn tenant_move_same_database(placement: &str) -> Error {
        Error::conflict(
            "tenant_move_same_database",
            "The placement `{placement}` is the same database as the tenant's current one; moving \
             would delete the tenant's data. Give each placement its own database.",
        )
        .with_param("placement", placement)
    }

    pub fn tenant_move_unreachable(placement: &str, error: &str) -> Error {
        Error::unavailable(
            "tenant_move_unreachable",
            "Cannot reach the database of the placement `{placement}`: {error}",
        )
        .with_param("placement", placement)
        .with_param("error", error)
    }

    pub fn tenant_move_storage_unavailable(placement: &str, error: &str) -> Error {
        Error::unavailable(
            "tenant_move_storage_unavailable",
            "Cannot reach the file storage of the placement `{placement}`: {error}",
        )
        .with_param("placement", placement)
        .with_param("error", error)
    }

    pub fn tenant_move_busy(jobs: u64) -> Error {
        Error::unavailable(
            "tenant_move_busy",
            "{jobs} job(s) or event(s) of the tenant are still being run at the old placement. \
             The move is paused; run it again in a few minutes.",
        )
        .with_param("jobs", jobs)
    }

    pub fn tenant_move_count_mismatch(table: &str, source: u64, target: u64) -> Error {
        Error::conflict(
            "tenant_move_count_mismatch",
            "After copying, `{table}` has {target} row(s) of the tenant at the new placement and \
             {source} at the old one; the tenant keeps its old placement. Run the move again to \
             copy once more.",
        )
        .with_param("table", table)
        .with_param("source", source)
        .with_param("target", target)
    }

    pub fn tenant_move_not_found(id: &str) -> Error {
        Error::not_found("tenant_move_not_found", "The tenant has no move `{id}`.")
            .with_param("id", id)
    }

    pub fn tenant_purge_current(placement: &str) -> Error {
        Error::conflict(
            "tenant_purge_current",
            "The placement `{placement}` keeps the tenant's live data; only a copy left on \
             another placement (by `--keep-source`) can be deleted.",
        )
        .with_param("placement", placement)
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
        TenantStatus::Deleted | TenantStatus::Merged => {
            Err(errors::tenant_not_found(&tenant.id().to_string()))
        }
    }
}

/// Đánh dấu route vẫn ghi được khi khách chỉ đọc: đăng nhập, làm mới / kết thúc phiên, đặt lại mật
/// khẩu, link trong thư (xác nhận, huỷ đăng ký — rút lại đồng ý luôn phải làm được), xuất dữ liệu.
/// Gắn bằng [`allowed_while_read_only`] trên router con chứa đúng các route đó.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowedWhileReadOnly;

/// Lớp gắn [`AllowedWhileReadOnly`] cho các route đã thêm vào router trước nó:
/// `OpenApiRouter::new().routes(routes!(login)).layer(allowed_while_read_only())`.
pub fn allowed_while_read_only() -> axum::Extension<AllowedWhileReadOnly> {
    axum::Extension(AllowedWhileReadOnly)
}

/// Kiểm request (`method`, `extensions`) có được làm với `tenant`: khách chỉ đọc chỉ nhận `GET`,
/// `HEAD`, `OPTIONS`, và route đánh dấu [`AllowedWhileReadOnly`]; còn lại `tenant_read_only`.
/// Kernel gọi ở chỗ giải khách của request (extractor `TenantContext`, `OptionalTenant`); tool
/// MCP kiểm theo cờ `read_only` của tool.
pub fn ensure_writable(
    tenant: &TenantContext,
    method: &axum::http::Method,
    extensions: &axum::http::Extensions,
) -> Result<(), Error> {
    use axum::http::Method;
    if !tenant.read_only()
        || [Method::GET, Method::HEAD, Method::OPTIONS].contains(method)
        || extensions.get::<AllowedWhileReadOnly>().is_some()
    {
        return Ok(());
    }
    Err(errors::tenant_read_only())
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
