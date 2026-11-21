//! Entity của bảng `auth_totp` và `auth_recovery_code` (SeaORM).

pub(crate) mod totp {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_totp")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách sở hữu dòng (bước 33).
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        pub secret: String,
        pub confirmed_at: Option<Timestamp>,
        pub last_used_step: Option<i64>,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod recovery_code {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_recovery_code")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách sở hữu dòng (bước 33).
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        pub code_hash: String,
        pub used_at: Option<Timestamp>,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}
