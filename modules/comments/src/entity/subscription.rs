//! Bảng `comments_reply_subscriptions`: email đã xác minh muốn nhận mail khi bình luận có trả
//! lời. Token huỷ của từng mail nằm ở `comments_tokens` (mục đích `unsubscribe`, D-130 của
//! `newsletter`: token riêng cho từng mail, chỉ lưu bản băm).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "comments_reply_subscriptions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Khách sở hữu dòng (bước 33).
    pub tenant_id: Uuid,
    /// Bình luận được theo dõi.
    pub comment_id: Uuid,
    /// Email đã xác minh (chữ thường).
    pub email: String,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}

impl autoxide_kernel::tenancy::TenantScoped for Entity {
    fn tenant_column() -> Self::Column {
        Column::TenantId
    }
}
