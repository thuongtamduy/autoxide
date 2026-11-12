//! Bảng `content_menu_items`: một mục của menu (cây nhiều tầng).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_menu_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub menu_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub sort_order: i32,
    pub label: String,
    pub target_entry_id: Option<Uuid>,
    pub target_term_id: Option<Uuid>,
    pub target_url: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
