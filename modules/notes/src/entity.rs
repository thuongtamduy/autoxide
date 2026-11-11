//! Entity của bảng `notes_note` (SeaORM). Chỉ dùng trong module này.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "notes_note")]
pub struct Model {
    // UUIDv7 sinh trong ứng dụng, không để database tự tăng.
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: String,
    pub body: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
