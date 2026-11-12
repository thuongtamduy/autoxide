//! Bảng `newsletter_campaigns`: một lần gửi bản tin tới một danh sách.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "newsletter_campaigns")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub list_id: Uuid,
    pub subject: String,
    pub preheader: Option<String>,
    pub body_text: String,
    pub body_html: Option<String>,
    /// Ngôn ngữ của chân thư: `en` hoặc `vi`.
    pub locale: String,
    /// Thư quảng cáo: nhãn đầu tiêu đề, giới hạn mỗi 24 giờ, bắt buộc địa chỉ người gửi.
    pub advertising: bool,
    /// `draft`, `scheduled`, `sending`, `paused`, `sent` hoặc `cancelled`.
    pub status: String,
    pub scheduled_at: Option<DateTimeUtc>,
    pub started_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
    /// Đã chụp xong danh sách người nhận vào `newsletter_deliveries`.
    pub prepared_at: Option<DateTimeUtc>,
    /// Lượt chạy: tăng mỗi lần gửi / hẹn giờ / tiếp tục. Job của lượt cũ thấy khác số thì thôi.
    pub run: i32,
    /// Nguồn tự động (vd `content.entry`) và khoá trong nguồn — unique: một bài một chiến dịch.
    pub source: Option<String>,
    pub source_ref: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
