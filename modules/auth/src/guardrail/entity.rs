//! Entity của bảng rào quyền (SeaORM).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "auth_tenant_guardrails")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách bị rào (rào áp cho nó và mọi khách cấp dưới của nó).
    pub tenant_id: Uuid,
    /// Khách cấp trên đã đặt rào.
    pub set_by_tenant_id: Uuid,
    /// Permission tối đa, tên cách nhau bằng dấu cách, theo thứ tự tên.
    pub max_permissions: String,
    pub updated_by: Uuid,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
