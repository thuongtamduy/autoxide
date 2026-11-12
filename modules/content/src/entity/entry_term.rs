//! Bảng `content_entry_terms`: mục mang term nào (term đầu là chính).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_entry_terms")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub entry_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub term_id: Uuid,
    pub sort_order: i32,
}

impl ActiveModelBehavior for ActiveModel {}
