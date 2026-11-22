//! Bảng `content_themes`: phần ghi đè token giao diện của một nút (bước 67).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_themes")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Nút của cây tổ chức (gốc = cả khách).
    pub org_id: Uuid,
    /// Object JSON tên token → giá trị (chỉ phần ghi đè).
    pub tokens: String,
    pub updated_by: Option<Uuid>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
