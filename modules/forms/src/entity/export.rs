//! Bảng `forms_exports`: bản xuất CSV chạy bằng job.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "forms_exports")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub form_id: Uuid,
    /// Người yêu cầu; tài khoản bị xoá thì `NULL`.
    pub requested_by: Option<Uuid>,
    /// Như tham số `status` của bản xuất đồng bộ; `NULL`: mọi trạng thái trừ `spam`.
    pub status_filter: Option<String>,
    /// `pending`, `running`, `ready` hoặc `failed`.
    pub state: String,
    pub row_count: Option<i64>,
    /// Số byte của file.
    pub size: Option<i64>,
    /// Khoá của file ở nơi lưu, khi đã ghi.
    pub storage_key: Option<String>,
    pub created_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    /// Sau lúc này job dọn dẹp xoá file và dòng.
    pub expires_at: Option<Timestamp>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
