//! Bảng `comments_reports`: báo cáo vi phạm; mỗi người báo một bình luận một lần.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "comments_reports")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub comment_id: Uuid,
    /// `user:<id>`, `ip:<dấu vết IP của khối /24 hay /48>`, `expired:<id báo cáo>` (dấu vết IP quá
    /// hạn) hoặc `deleted:<id báo cáo>` (tài khoản bị xoá); unique cùng `comment_id`.
    pub reporter_key: String,
    pub reporter_user_id: Option<Uuid>,
    /// `spam`, `abuse`, `illegal` hoặc `other`.
    pub reason: String,
    /// Ghi chú của người báo (chỉ người kiểm duyệt xem).
    pub note: Option<String>,
    pub created_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
