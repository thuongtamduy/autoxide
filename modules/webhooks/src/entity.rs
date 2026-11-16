//! Entity của bảng `webhooks_endpoint` và `webhooks_delivery` (SeaORM).

pub(crate) mod endpoint {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "webhooks_endpoint")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách sở hữu endpoint (bước 33).
        pub tenant_id: Uuid,
        pub url: String,
        pub description: String,
        /// Secret ký, đã mã hoá (`sealed:v1:…`, D-278); chỉ mở lúc ký.
        pub secret: String,
        /// JSON: danh sách tên event.
        pub events: String,
        pub disabled_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}

pub(crate) mod delivery {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "webhooks_delivery")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        /// Khách của endpoint (bước 33).
        pub tenant_id: Uuid,
        pub endpoint_id: Uuid,
        pub event_id: Uuid,
        pub event_name: String,
        pub body: String,
        pub status: String,
        pub attempts: i32,
        pub response_status: Option<i32>,
        pub last_error: Option<String>,
        pub created_at: DateTimeUtc,
        pub last_attempt_at: Option<DateTimeUtc>,
        pub delivered_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}
