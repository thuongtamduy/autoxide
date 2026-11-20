//! Bảng `customers_profiles`.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_profiles")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub user_id: Option<Uuid>,
    pub phone_key: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub email_lower: Option<String>,
    pub full_name: Option<String>,
    pub full_name_search: Option<String>,
    pub birthday: Option<Date>,
    pub gender: Option<String>,
    pub tier_code: Option<String>,
    pub currency: Option<String>,
    pub total_spent_minor: i64,
    pub orders_count: i64,
    pub first_order_at: Option<DateTimeUtc>,
    pub last_order_at: Option<DateTimeUtc>,
    pub custom_fields: String,
    pub birthday_md: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
