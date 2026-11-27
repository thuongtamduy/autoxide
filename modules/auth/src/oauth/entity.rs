//! Entity của ba bảng OAuth (SeaORM).

pub(crate) mod client {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_oauth_clients")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub client_id: String,
        pub client_name: String,
        /// JSON: danh sách `redirect_uris`.
        pub redirect_uris: String,
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

pub(crate) mod grant {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_oauth_grants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        pub client_id: String,
        pub client_name: String,
        pub registration: String,
        pub redirect_uri: String,
        pub resource: String,
        pub code_challenge: String,
        /// Permission đã duyệt, cách nhau bằng dấu cách.
        pub scope: String,
        pub created_at: Timestamp,
        pub expires_at: Timestamp,
        pub last_used_at: Option<Timestamp>,
        pub revoked_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }

    impl Model {
        /// Danh sách permission đã duyệt.
        pub(crate) fn permissions(&self) -> Vec<String> {
            self.scope.split_whitespace().map(str::to_owned).collect()
        }
    }
}

pub(crate) mod token {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "auth_oauth_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub grant_id: Uuid,
        pub kind: String,
        pub token_hash: String,
        pub expires_at: Timestamp,
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
