//! Bảng `content_embedding_state`: một dòng mỗi (`scope`, `locale`) với số `version` tăng mỗi lần
//! vector đổi — mỗi máy chủ giữ vector trong bộ nhớ và chỉ nạp lại khi số này đổi.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_embedding_state")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub scope: String,
    pub locale: String,
    pub version: i64,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
