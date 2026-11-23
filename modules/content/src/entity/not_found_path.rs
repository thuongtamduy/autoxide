//! Bảng `content_not_found_paths`: số lượt hỏi đường dẫn không có, theo website và ngày (bước 67).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_not_found_paths")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub path_hash: String,
    pub locale: String,
    pub path: String,
    /// `YYYY-MM-DD` (UTC).
    pub day: String,
    pub hits: i64,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
