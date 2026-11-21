//! Entity của hai bảng passkey (SeaORM).

pub(crate) mod passkey {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_passkey")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách sở hữu dòng (bước 33).
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        pub credential_id: Vec<u8>,
        pub user_handle: Vec<u8>,
        pub static_state: Vec<u8>,
        pub dynamic_state: Vec<u8>,
        pub name: String,
        pub created_at: Timestamp,
        pub last_used_at: Option<Timestamp>,
        /// Loại authenticator nó tự khai lúc đăng ký (D-272); `None` khi không biết.
        pub aaguid: Option<Uuid>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod ceremony {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_passkey_ceremony")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách sở hữu dòng (bước 33).
        pub tenant_id: Uuid,
        pub user_id: Option<Uuid>,
        pub state: Vec<u8>,
        pub expires_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}
