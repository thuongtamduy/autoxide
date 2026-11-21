//! Bảng `menus_menus`: một menu theo (`key`, `locale`).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "menus_menus")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Website (nút của cây tổ chức) của menu (bước 34); gốc = ID của khách. Luôn có giá trị.
    pub org_id: Option<Uuid>,
    pub key: String,
    pub locale: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
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
