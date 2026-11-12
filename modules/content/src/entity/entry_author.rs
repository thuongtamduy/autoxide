//! Bảng `content_entry_authors`: tác giả của mục (người đầu là chính).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_entry_authors")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub entry_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub author_id: Uuid,
    pub sort_order: i32,
}

impl ActiveModelBehavior for ActiveModel {}
