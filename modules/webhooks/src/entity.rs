//! Entity của bảng `webhooks_endpoint` và `webhooks_delivery` (SeaORM).

pub(crate) mod endpoint {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "webhooks_endpoint")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub url: String,
        pub description: String,
        pub secret: String,
        /// JSON: danh sách tên event.
        pub events: String,
        pub disabled_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod delivery {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "webhooks_delivery")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
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
}
