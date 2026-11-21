//! Bảng `customers_referral_rules`.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "customers_referral_rules")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub referrer_points: i64,
    pub referee_points: i64,
    pub referrer_promotion_id: Option<Uuid>,
    pub referee_promotion_id: Option<Uuid>,
    pub min_first_order_minor: i64,
    pub currency: String,
    pub effective_from: Timestamp,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
