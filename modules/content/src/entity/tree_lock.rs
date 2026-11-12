//! Bảng `content_tree_locks`: một dòng mỗi ngôn ngữ; mọi thay đổi cây trang của ngôn ngữ đó
//! cập nhật dòng này trước, nên chạy lần lượt.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_tree_locks")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub locale: String,
    pub version: i64,
}

impl ActiveModelBehavior for ActiveModel {}
