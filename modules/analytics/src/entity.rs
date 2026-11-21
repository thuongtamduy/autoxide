//! Bảng của module `analytics`; mọi bảng là cấp khách hàng (`tenant_id`).

macro_rules! tenant_scoped {
    () => {
        impl autoxide_kernel::tenancy::TenantScoped for Entity {
            fn tenant_column() -> Self::Column {
                Column::TenantId
            }
        }
    };
}

pub(crate) mod event {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "analytics_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub site_org_id: Uuid,
        pub occurred_at: Timestamp,
        /// Ngày UTC `YYYY-MM-DD`.
        pub day: String,
        #[sea_orm(column_name = "type")]
        pub kind: String,
        pub session_key: String,
        pub user_id: Option<Uuid>,
        pub path: String,
        pub referrer_host: Option<String>,
        pub source: String,
        pub landing: bool,
        pub utm_source: Option<String>,
        pub utm_medium: Option<String>,
        pub utm_campaign: Option<String>,
        pub utm_term: Option<String>,
        pub utm_content: Option<String>,
        pub object: Option<String>,
        /// JSON.
        pub props: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod daily {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "analytics_daily")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub site_org_id: Uuid,
        pub day: String,
        pub dimension: String,
        pub value: String,
        pub views: i64,
        pub visitors: i64,
        pub events: i64,
        pub bounces: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod funnel_daily {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "analytics_funnel_daily")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub site_org_id: Uuid,
        pub day: String,
        pub funnel: String,
        pub step: i32,
        pub step_key: String,
        pub sessions: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod day_key {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "analytics_day_keys")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub day: String,
        pub key: String,
        pub created_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod rollup {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "analytics_rollups")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub day: String,
        pub rolled_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
