//! Bảng `newsletter_subscriptions`: người đăng ký nào ở danh sách nào, trạng thái riêng của từng
//! danh sách. Khoá chính (`subscriber_id`, `list_id`).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "newsletter_subscriptions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub subscriber_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub list_id: Uuid,
    /// `unconfirmed`, `confirmed` hoặc `unsubscribed`.
    pub status: String,
    /// JSON bằng chứng đồng ý `{text, version, at, source, ip_fingerprint}`, nếu có.
    pub consent: Option<String>,
    /// Lý do bộ chống spam giữ lại (`too_fast,duplicate`…): đăng ký bị buộc xác nhận qua mail.
    pub hold_reason: Option<String>,
    pub requested_at: DateTimeUtc,
    pub confirmed_at: Option<DateTimeUtc>,
    pub unsubscribed_at: Option<DateTimeUtc>,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
