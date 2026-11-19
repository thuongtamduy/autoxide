//! Hợp đồng của module `organizations` cho module khác (`autoxide_organizations::api::…`). Lấy
//! trait lúc khởi động: `context.require::<dyn OrgDirectory>()?`; module đó đăng ký SAU
//! `organizations` trong `crates/app/src/modules.rs`.
//!
//! Lọc dữ liệu theo nhánh không cần module này: dùng `Principal::scope_condition` /
//! `require_at` và `OrgTree` của kernel. Trait ở đây cho phần nghĩa của nút (tên, loại, trạng
//! thái) — vd `auth` kiểm nút có thật khi cấp membership.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext};

/// Trạng thái của một nút. Chỉ nút `active` (và mọi nút trên nó `active`) mới ghi dữ liệu được.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnitStatus {
    /// Chờ người có `organizations.units.manage` duyệt.
    Pending,
    Active,
    Suspended,
    Archived,
}

impl UnitStatus {
    /// Tên ổn định: `pending`, `active`, `suspended`, `archived`.
    pub const fn as_str(self) -> &'static str {
        match self {
            UnitStatus::Pending => "pending",
            UnitStatus::Active => "active",
            UnitStatus::Suspended => "suspended",
            UnitStatus::Archived => "archived",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pending" => Some(UnitStatus::Pending),
            "active" => Some(UnitStatus::Active),
            "suspended" => Some(UnitStatus::Suspended),
            "archived" => Some(UnitStatus::Archived),
            _ => None,
        }
    }
}

/// Một nút của cây tổ chức.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Unit {
    pub id: OrgId,
    /// `None` chỉ ở nút gốc.
    pub parent_id: Option<OrgId>,
    pub unit_type: String,
    pub name: String,
    pub code: Option<String>,
    pub status: UnitStatus,
}

impl Unit {
    /// Một nút đang hoạt động (cho cài đặt `OrgDirectory` giả trong test của module khác).
    pub fn new(
        id: OrgId,
        parent_id: Option<OrgId>,
        unit_type: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Unit {
            id,
            parent_id,
            unit_type: unit_type.into(),
            name: name.into(),
            code: None,
            status: UnitStatus::Active,
        }
    }
}

/// Giá trị hiệu lực của một cài đặt ở một nút (cài đặt kế thừa theo cây, bước 34 mục 4).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EffectiveSetting {
    pub key: String,
    /// `None`: không nút nào đặt và cài đặt không có mặc định.
    pub value: Option<serde_json::Value>,
    /// Nút đã đặt giá trị; `None`: giá trị mặc định.
    pub source_unit_id: Option<OrgId>,
    /// Giá trị đến từ nút khác (cấp trên) hay mặc định, không do chính nút đặt.
    pub inherited: bool,
}

impl EffectiveSetting {
    /// Giá trị hiệu lực (cho cài đặt `OrgDirectory` giả trong test của module khác).
    pub fn new(
        key: impl Into<String>,
        value: Option<serde_json::Value>,
        source_unit_id: Option<OrgId>,
        inherited: bool,
    ) -> Self {
        EffectiveSetting { key: key.into(), value, source_unit_id, inherited }
    }
}

/// Giờ mở cửa của một ngày trong tuần, giờ địa phương của nút (hồ sơ công khai, bước 34).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpeningRange {
    /// Ngày trong tuần theo ISO: 1 thứ Hai … 7 Chủ nhật.
    pub day: u8,
    pub opens: chrono::NaiveTime,
    pub closes: chrono::NaiveTime,
}

impl OpeningRange {
    pub fn new(day: u8, opens: chrono::NaiveTime, closes: chrono::NaiveTime) -> Self {
        OpeningRange { day, opens, closes }
    }
}

/// Tra nút của cây tổ chức.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait OrgDirectory: Send + Sync {
    /// Nút `id` của khách, hoặc `None`.
    async fn unit(&self, tenant: &TenantContext, id: OrgId) -> Result<Option<Unit>, Error>;
    /// Nút gốc của khách (tạo nếu còn thiếu — chịu được event "khách mới" đến chậm).
    async fn root(&self, tenant: &TenantContext) -> Result<Unit, Error>;
    /// Các nút trên `id`, gần nhất trước, gồm gốc; không gồm chính nó.
    async fn ancestors(&self, tenant: &TenantContext, id: OrgId) -> Result<Vec<Unit>, Error>;
    /// `id` có thuộc nhánh của `root` (gồm chính `root`) không.
    async fn is_within(
        &self,
        tenant: &TenantContext,
        id: OrgId,
        root: OrgId,
    ) -> Result<bool, Error>;
    /// Giá trị hiệu lực của cài đặt `key` ở nút `id` (nút gần nhất trên chuỗi có đặt, hay mặc
    /// định). Cài đặt không ai khai → `organizations.setting_unknown`.
    async fn setting(
        &self,
        tenant: &TenantContext,
        id: OrgId,
        key: &str,
    ) -> Result<Option<EffectiveSetting>, Error>;

    /// Giờ mở cửa trong hồ sơ công khai của nút `id`; `None` khi nút chưa khai (bên gọi dùng giờ
    /// mặc định của mình). Mặc định: `None`.
    async fn opening_hours(
        &self,
        _tenant: &TenantContext,
        _id: OrgId,
    ) -> Result<Option<Vec<OpeningRange>>, Error> {
        Ok(None)
    }
}

/// Nhóm nút ngoài cây (bước 38, D-232): `organizations` công bố, `reporting` lấy bằng
/// `context.require::<dyn OrgGroups>()` để lọc / gom báo cáo. Nhóm không cấp quyền.
#[async_trait::async_trait]
pub trait OrgGroups: Send + Sync {
    /// Nút thành viên của nhóm `group`; `None` khi khách không có nhóm này. KHÔNG lọc theo quyền —
    /// bên gọi giao với phạm vi của người xem.
    async fn members(
        &self,
        tenant: &TenantContext,
        group: uuid::Uuid,
    ) -> Result<Option<Vec<OrgId>>, Error>;
}
