//! Bảng `newsletter_deliveries`: một dòng cho mỗi người nhận của một chiến dịch — gửi tiếp đúng
//! chỗ khi bị ngắt, không gửi trùng khi job chạy lại.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "newsletter_deliveries")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub subscriber_id: Uuid,
    /// `pending`, `sent`, `failed` hoặc `skipped`.
    pub status: String,
    pub attempts: i32,
    /// SHA-256 của token huỷ đăng ký trong mail này (token chỉ có trong mail). Có giá trị
    /// nghĩa là một worker đã nhận gửi dòng này.
    pub unsubscribe_token_hash: Option<String>,
    pub claimed_at: Option<DateTimeUtc>,
    pub sent_at: Option<DateTimeUtc>,
    /// Lý do `skipped` / `failed` (`suppressed`, `unsubscribed`, `hard_bounce`…).
    pub error: Option<String>,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
