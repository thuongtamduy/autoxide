//! Entity của các bảng SCIM (SeaORM). Mỗi bảng một module con vì SeaORM đặt tên cố định.

pub(crate) mod token {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_scim_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub name: String,
        pub prefix: String,
        pub token_hash: String,
        pub created_by: Uuid,
        pub created_at: Timestamp,
        pub last_used_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod scim_user {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_scim_users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        /// ID tài khoản (`auth_user.id`).
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_name: String,
        pub user_name_key: String,
        pub external_id: Option<String>,
        pub external_key: Option<String>,
        pub given_name: Option<String>,
        pub family_name: Option<String>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod group {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_scim_groups")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub display_name: String,
        pub display_name_key: String,
        pub external_id: Option<String>,
        pub external_key: Option<String>,
        pub created_at: Timestamp,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod member {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_scim_group_members")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub group_id: Uuid,
        pub user_id: Uuid,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod mapping {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_scim_group_mappings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub group_id: Uuid,
        pub org_id: Uuid,
        pub role_id: Uuid,
        pub created_by: Uuid,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod grant {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_scim_grants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_role_id: Uuid,
        pub user_id: Uuid,
        pub org_id: Uuid,
        pub role_id: Uuid,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}
