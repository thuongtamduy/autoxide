//! Bảng `comments_comments`: một bình luận (gốc hay trả lời).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "comments_comments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Loại đích (`content.entry`…) — chỉ là chuỗi, không foreign key.
    pub target_type: String,
    pub target_id: String,
    /// Bình luận gốc của luồng; bình luận gốc trỏ về chính nó.
    pub root_id: Uuid,
    /// Bình luận gốc (với trả lời); `None` với bình luận gốc.
    pub parent_id: Option<Uuid>,
    /// Bình luận đang được trả lời (bình luận gốc hay một trả lời khác cùng luồng).
    pub reply_to_id: Option<Uuid>,
    /// 0: bình luận gốc; 1: trả lời.
    pub depth: i32,
    /// `user` hoặc `guest`.
    pub author_kind: String,
    pub user_id: Option<Uuid>,
    /// Tên khách tự khai; người dùng đăng nhập lấy tên ở `comments_profiles`.
    pub author_name: Option<String>,
    /// Email (trim + chữ thường) để báo trả lời; không bao giờ trả ra công khai.
    pub author_email: Option<String>,
    /// Email đã xác minh bằng link trong mail.
    pub email_verified: bool,
    /// Chữ thường, đã lọc.
    pub body: String,
    /// `pending`, `published`, `rejected`, `spam` hoặc `deleted`.
    pub status: String,
    /// Lý do giữ lại (`moderation`, `too_many_links`, `too_fast,duplicate`…).
    pub hold_reason: Option<String>,
    /// SHA-256 của token sửa của khách.
    pub edit_token_hash: Option<String>,
    pub edited_at: Option<DateTimeUtc>,
    /// Lần đầu được hiện.
    pub published_at: Option<DateTimeUtc>,
    /// `ip_hash::fingerprint`; xoá sau `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS`.
    pub ip_fingerprint: Option<String>,
    /// `en` hoặc `vi` (ngôn ngữ của mail gửi cho tác giả).
    pub locale: Option<String>,
    /// `antispam::content_hash` của chữ, để dò bài trùng.
    pub content_hash: String,
    /// `search::fold` của chữ và tên, để tìm.
    pub search_key: String,
    /// Số trả lời đang hiện (chỉ bình luận gốc), cập nhật cùng transaction.
    pub reply_count: i32,
    /// Số báo cáo vi phạm.
    pub report_count: i32,
    /// Lúc bị giữ lại vì đủ báo cáo — chỉ một lần.
    pub reports_held_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
