//! Bảng `newsletter_lists`: danh sách bản tin.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "newsletter_lists")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// Slug, unique: website gửi khi đăng ký.
    pub key: String,
    /// JSON `{en, vi}`.
    pub name: String,
    /// JSON `{en, vi}`.
    pub description: String,
    /// Đăng ký công khai phải xác nhận qua mail.
    pub double_opt_in: bool,
    /// Người lạ thấy và đăng ký được.
    pub public: bool,
    /// Chiến dịch cho bài mới: `off`, `draft` hoặc `send`.
    pub new_posts: String,
    /// Chỉ bài mới bằng ngôn ngữ này (`en`/`vi`); `None`: mọi ngôn ngữ.
    pub new_post_locale: Option<String>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
