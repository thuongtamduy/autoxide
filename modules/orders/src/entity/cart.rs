//! Bảng `orders_carts`: một giỏ hàng (người mua đăng nhập hay vãng lai).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_carts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub buyer_user_id: Option<Uuid>,
    pub token_hash: Option<String>,
    pub currency: String,
    pub locale: String,
    pub codes: String,
    pub voucher_ids: String,
    pub gift_selections: String,
    pub ship_to: Option<String>,
    pub shipping_option: Option<String>,
    pub payment_method: Option<String>,
    pub attribution: Option<String>,
    pub status: String,
    pub merged_into: Option<Uuid>,
    pub active_buyer_key: Option<String>,
    pub abandoned_notified_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub expires_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
