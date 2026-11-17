//! Entity của hai bảng API key (SeaORM).

pub(crate) mod api_key {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_api_key")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách sở hữu dòng (bước 33).
        pub tenant_id: Uuid,
        pub user_id: Uuid,
        pub name: String,
        pub prefix: String,
        pub key_hash: String,
        pub expires_at: Option<DateTimeUtc>,
        /// Nút của hàng rào (bước 34); nút gốc = không rào. Luôn có giá trị.
        pub org_id: Option<Uuid>,
        pub last_used_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
        /// `human`, `agent` hay `service` (bước 51 P12).
        pub kind: String,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod api_key_permission {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_api_key_permission")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách sở hữu dòng (bước 33).
        pub tenant_id: Uuid,
        pub api_key_id: Uuid,
        pub permission: String,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}
