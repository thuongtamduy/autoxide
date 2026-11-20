//! Bảng `orders_pos_shifts`: ca bán hàng của một quầy (tiền đầu ca, tiền đếm được, chênh lệch).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_pos_shifts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub register_code: String,
    pub open_key: Option<String>,
    pub opened_by: Uuid,
    pub opened_at: DateTimeUtc,
    pub currency: String,
    pub opening_cash_minor: i64,
    pub closed_by: Option<Uuid>,
    pub closed_at: Option<DateTimeUtc>,
    pub expected_cash_minor: Option<i64>,
    pub counted_cash_minor: Option<i64>,
    pub variance_minor: Option<i64>,
    pub variance_reason: Option<String>,
    pub variance_note: Option<String>,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTimeUtc>,
    pub status: String,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
