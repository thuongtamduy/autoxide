//! Entity của hai bảng passkey (SeaORM).

pub(crate) mod passkey {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_passkey")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        #[sea_orm(unique)]
        pub credential_id: Vec<u8>,
        pub user_handle: Vec<u8>,
        pub static_state: Vec<u8>,
        pub dynamic_state: Vec<u8>,
        pub name: String,
        pub created_at: DateTimeUtc,
        pub last_used_at: Option<DateTimeUtc>,
        /// Loại authenticator nó tự khai lúc đăng ký (D-272); `None` khi không biết.
        pub aaguid: Option<Uuid>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod ceremony {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_passkey_ceremony")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Option<Uuid>,
        pub state: Vec<u8>,
        pub expires_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}
