//! Entity của hai bảng API key (SeaORM).

pub(crate) mod api_key {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_api_key")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        pub name: String,
        pub prefix: String,
        #[sea_orm(unique)]
        pub key_hash: String,
        pub expires_at: Option<DateTimeUtc>,
        pub last_used_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod api_key_permission {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_api_key_permission")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub api_key_id: Uuid,
        pub permission: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}
