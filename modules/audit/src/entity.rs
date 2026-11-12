//! Entity của bảng `audit_entry` (SeaORM).

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "audit_entry")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub module: String,
    pub action: String,
    pub actor_user_id: Option<Uuid>,
    pub actor_api_key_id: Option<Uuid>,
    pub actor_kind: Option<String>,
    pub subject: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub details: Option<String>,
    pub occurred_at: DateTimeUtc,
    pub recorded_at: DateTimeUtc,
}

impl ActiveModelBehavior for ActiveModel {}
