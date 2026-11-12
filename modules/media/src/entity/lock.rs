//! Bảng `media_lock`: một dòng `folder_tree`, cập nhật đầu mỗi lệnh chuyển thư mục để các
//! lệnh đó chạy lần lượt (D-086).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "media_lock")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub name: String,
    pub counter: i64,
}

impl ActiveModelBehavior for ActiveModel {}
