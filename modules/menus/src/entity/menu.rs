//! Bảng `menus_menus`: một menu theo (`key`, `locale`).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "menus_menus")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    pub key: String,
    pub locale: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    /// Người tạo hay thay cây mục gần nhất; `None`: không biết (menu trước bản này, hay lệnh
    /// chạy bằng hệ thống).
    pub updated_by: Option<Uuid>,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
