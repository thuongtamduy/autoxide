//! Bảng `orders_review_summaries`: bản nháp tóm tắt đánh giá bằng AI chờ nhân viên duyệt (D-109).

use autoxide_kernel::time::Timestamp;
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "orders_review_summaries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub product_id: Uuid,
    pub locale: String,
    pub approved_key: Option<String>,
    pub body: Option<String>,
    pub themes: String,
    pub status: String,
    pub source_review_count: i32,
    pub model: Option<String>,
    pub requested_by: Uuid,
    pub generated_at: Option<Timestamp>,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
