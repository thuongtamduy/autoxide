//! Bảng `comments_bans`: cấm bình luận theo người dùng, email hay dấu vết IP.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "comments_bans")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    /// `user`, `email` hoặc `ip`.
    pub kind: String,
    /// ID người dùng, email (chữ thường) hay `ip_fingerprint`.
    pub value: String,
    pub reason: String,
    /// Hết cấm lúc này; `None`: vĩnh viễn.
    pub until: Option<DateTimeUtc>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
