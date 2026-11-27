//! Bảng `notes_note` (lớp `db` của kernel, D-20261126-kernel-data-layer). Chỉ dùng trong module này.

use autoxide_kernel::db::{Record, TenantKey};
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

/// Một ghi chú.
#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "notes_note", scope = tenant)]
pub(crate) struct Note {
    // UUIDv7 sinh trong ứng dụng, không để database tự tăng.
    #[record(primary_key)]
    pub id: Uuid,
    /// Khách sở hữu ghi chú (bước 33): handle `tenant.db()` tự lọc theo cột này.
    pub tenant_id: TenantKey,
    /// Nút sở hữu ghi chú (bước 34); nút gốc = ID của khách. Luôn có giá trị (cột NULL được).
    pub org_id: Option<Uuid>,
    pub owner_id: Uuid,
    pub title: String,
    pub body: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
