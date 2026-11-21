//! Bảng `orders_warranty_claims`: phiếu tiếp nhận bảo hành và trạng thái xử lý.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_warranty_claims")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub warranty_id: Uuid,
    pub org_id: Uuid,
    pub number: String,
    pub status: String,
    pub issue: String,
    pub resolution: Option<String>,
    pub due_at: Timestamp,
    pub created_by: Uuid,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
