//! Bảng `customers_gift_cards`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_gift_cards")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_scope: Option<Uuid>,
    pub code_hash: String,
    pub code_last4: String,
    pub initial_minor: i64,
    pub balance_minor: i64,
    pub currency: String,
    pub status: String,
    pub expires_at: Option<DateTimeUtc>,
    pub owner_profile_id: Option<Uuid>,
    pub issued_reference: String,
    pub issued_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
