//! Bảng mặt điều khiển `address_dataset`, `address_unit` (không có `tenant_id`), qua lớp `db` của kernel
//! (D-20261126-kernel-data-layer). Chỉ dùng trong module này.

use autoxide_kernel::db::Record;
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

/// Một bộ dữ liệu đã nhập (một văn bản gốc).
#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "address_dataset", scope = global)]
pub(crate) struct Dataset {
    #[record(primary_key)]
    pub id: Uuid,
    pub country: String,
    /// Ngày bộ dữ liệu có hiệu lực (`YYYY-MM-DD`).
    pub effective_from: String,
    /// Văn bản gốc, vd "QĐ 19/2025/QĐ-TTg".
    pub title: String,
    pub source_url: String,
    pub retrieved_on: String,
    pub sha256: String,
    pub province_count: i32,
    pub commune_count: i32,
    pub imported_at: Timestamp,
}

/// Một đơn vị hành chính trong một khoảng hiệu lực.
#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "address_unit", scope = global)]
pub(crate) struct Unit {
    #[record(primary_key)]
    pub id: Uuid,
    pub country: String,
    /// 1: tỉnh; 2: xã.
    pub level: i32,
    /// Mã của văn bản (giữ số 0 đầu: lưu chuỗi).
    pub code: String,
    pub name: String,
    /// `search::fold(name)`: tìm không dấu.
    pub name_folded: String,
    pub kind: String,
    pub parent_code: Option<String>,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub dataset_id: Uuid,
}
