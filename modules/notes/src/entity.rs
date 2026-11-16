//! Entity của bảng `notes_note` (SeaORM). Chỉ dùng trong module này.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "notes_note")]
pub struct Model {
    // UUIDv7 sinh trong ứng dụng, không để database tự tăng.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu ghi chú (bước 33): mọi truy vấn lọc theo cột này (`.scoped`).
    pub tenant_id: Uuid,
    /// Nút sở hữu ghi chú (bước 34); nút gốc = ID của khách. Luôn có giá trị (cột NULL được).
    pub org_id: Option<Uuid>,
    pub owner_id: Uuid,
    pub title: String,
    pub body: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
