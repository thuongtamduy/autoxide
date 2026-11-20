//! Bảng `orders_review_stats`: thống kê sao của đánh giá đang công khai theo sản phẩm.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_review_stats")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub product_id: Uuid,
    pub review_count: i64,
    pub rating_sum: i64,
    pub count_1: i64,
    pub count_2: i64,
    pub count_3: i64,
    pub count_4: i64,
    pub count_5: i64,
    pub media_count: i64,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
