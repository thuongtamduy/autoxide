//! Bảng `idempotency_record` (lớp `db` của kernel, D-20261126-kernel-data-layer). Chỉ dùng trong module này.

use autoxide_kernel::db::{Record, TenantKey};
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

/// Kết quả đã lưu (hay đang chạy) của một idempotency key.
#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "idempotency_record", scope = tenant)]
pub(crate) struct IdempotencyRecord {
    #[record(primary_key)]
    pub id: Uuid,
    /// Khách của request (bước 33): handle `tenant.db()` tự lọc theo cột này.
    pub tenant_id: TenantKey,
    pub scope: String,
    pub key: String,
    pub fingerprint: String,
    pub status: Option<i32>,
    pub headers: Option<String>,
    pub body: Option<Vec<u8>>,
    pub locked_until: Timestamp,
    pub expires_at: Timestamp,
    pub created_at: Timestamp,
}
