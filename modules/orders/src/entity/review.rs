//! Bảng `orders_reviews`: đánh giá của người đã nhận hàng, một đánh giá mỗi dòng đơn.

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_reviews")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Uuid,
    pub order_id: Uuid,
    pub order_line_id: Uuid,
    pub product_id: Uuid,
    pub variant_id: Uuid,
    pub author_user_id: Uuid,
    pub rating: i32,
    pub body: String,
    pub media_ids: String,
    pub has_media: bool,
    pub status: String,
    pub hidden_reason: Option<String>,
    pub hidden_by: Option<Uuid>,
    pub hidden_at: Option<Timestamp>,
    pub reply: Option<String>,
    pub replied_by: Option<Uuid>,
    pub replied_at: Option<Timestamp>,
    pub helpful_count: i32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
