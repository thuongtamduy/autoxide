//! Entity của bảng `idempotency_record` (SeaORM).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "idempotency_record")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách của request (bước 33): mọi truy vấn lọc theo cột này (`.scoped`).
    pub tenant_id: Uuid,
    pub scope: String,
    pub key: String,
    pub fingerprint: String,
    pub status: Option<i32>,
    pub headers: Option<String>,
    pub body: Option<Vec<u8>>,
    pub locked_until: DateTimeUtc,
    pub expires_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
