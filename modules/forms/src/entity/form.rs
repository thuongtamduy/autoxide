//! Bảng `forms_forms`: định nghĩa form hiện hành. JSON lưu dạng chữ.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "forms_forms")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Slug, unique, nằm trong URL công khai.
    pub key: String,
    pub name: String,
    /// `open` hoặc `closed`.
    pub status: String,
    /// JSON: danh sách trường của phiên bản hiện hành (xem `forms/fields.rs`).
    pub fields: String,
    /// Phiên bản hiện hành của danh sách trường (bắt đầu từ 1).
    pub version: i32,
    /// JSON: danh sách `scheme://host[:port]` được gửi form từ đó; rỗng là mọi nơi.
    pub allowed_origins: String,
    /// JSON `{en, vi}`.
    pub success_message: String,
    /// JSON: email nhận thông báo. KHÔNG BAO GIỜ trả ra endpoint công khai.
    pub notify_emails: String,
    /// `key` của trường `email` dùng làm `Reply-To` và để tra theo email.
    pub reply_to_field: Option<String>,
    /// Giới hạn bài gửi mỗi giờ theo khối IP; `None`: theo cấu hình.
    pub ip_limit_per_hour: Option<i32>,
    /// Giới hạn bài gửi mỗi giờ theo email; `None`: theo cấu hình.
    pub email_limit_per_hour: Option<i32>,
    /// Số ngày giữ bài gửi; `None`: theo cấu hình; `0`: giữ mãi.
    pub retention_days: Option<i32>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
