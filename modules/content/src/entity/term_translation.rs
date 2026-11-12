//! Bảng `content_term_translations`: tên, slug, mô tả của term theo ngôn ngữ.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "content_term_translations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub term_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub locale: String,
    pub taxonomy: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub meta_title: String,
    pub meta_description: String,
}

impl ActiveModelBehavior for ActiveModel {}
