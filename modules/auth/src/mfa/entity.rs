//! Entity của bảng `auth_totp` (SeaORM).

pub(crate) mod totp {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_totp")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        #[sea_orm(unique)]
        pub user_id: Uuid,
        pub secret: String,
        pub confirmed_at: Option<DateTimeUtc>,
        pub last_used_step: Option<i64>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}
