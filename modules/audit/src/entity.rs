//! Entity của bảng `audit_entry` và `audit_chain` (SeaORM).

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
    /// Vị trí trong chuỗi băm (D-183). Có ở mọi dòng sau migration `m0004`.
    pub sequence: Option<i64>,
    pub payload_hash: Option<String>,
    pub chain_hash: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}

/// Bảng `audit_chain`: một dòng (`name` = [`HEAD`](crate::chain::HEAD)) giữ số thứ tự và mã băm
/// của dòng cuối trong chuỗi.
pub(crate) mod head {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "audit_chain")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub name: String,
        pub sequence: i64,
        pub hash: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}
