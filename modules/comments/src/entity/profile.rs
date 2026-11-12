//! Bảng `comments_profiles`: tên hiển thị người dùng tự đặt. `Principal` không có tên và module
//! không đọc `auth` (D-046), nên tên nằm ở đây; `user_id` không có foreign key.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "comments_profiles")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub user_id: Uuid,
    pub display_name: String,
    /// `words_key` của tên, unique: hai người không trùng tên (chống mạo danh).
    pub name_key: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
