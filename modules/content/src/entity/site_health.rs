//! Bảng `content_site_health`: lần kiểm sức khoẻ gần nhất của một website (bước 67).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_site_health")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Website (nút của cây tổ chức).
    pub org_id: Uuid,
    /// JSON: danh sách phát hiện.
    pub findings: String,
    pub findings_count: i32,
    pub checked_by: Option<Uuid>,
    pub checked_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
