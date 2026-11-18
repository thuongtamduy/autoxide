//! Entity (SeaORM) của các bảng `shipping_…` — mọi bảng là cấp khách hàng (`tenant_id`). Chỉ dùng
//! trong module này.

/// `impl TenantScoped` cho entity có cột `tenant_id`.
macro_rules! tenant_scoped {
    () => {
        impl autoxide_kernel::tenancy::TenantScoped for Entity {
            fn tenant_column() -> Self::Column {
                Column::TenantId
            }
        }
    };
}

pub(crate) mod address {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_addresses")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub buyer_id: Uuid,
        pub label: String,
        pub recipient_name: String,
        pub phone: String,
        pub country: String,
        pub province_code: String,
        pub province_name: String,
        pub commune_code: String,
        pub commune_name: String,
        pub line: String,
        pub postal_code: Option<String>,
        pub is_default: bool,
        pub default_key: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod zone {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_zones")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub name: String,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod zone_region {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_zone_regions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub zone_id: Uuid,
        pub country: String,
        /// `None` = cả nước.
        pub province_code: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod method {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_methods")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub code: String,
        pub kind: String,
        pub names: String,
        pub active: bool,
        pub position: i32,
        pub carrier_account_id: Option<Uuid>,
        pub service_code: Option<String>,
        pub free_over_minor: Option<i64>,
        pub handling_fee_minor: i64,
        pub currency: String,
        pub tax_category: String,
        pub volumetric_divisor: i32,
        pub default_item_weight_grams: Option<i64>,
        pub countries: String,
        pub estimated_min_days: Option<i16>,
        pub estimated_max_days: Option<i16>,
        pub calendar_id: Option<Uuid>,
        pub cutoff_minute: Option<i32>,
        pub same_day: bool,
        pub hold_days: Option<i16>,
        pub auto_book: bool,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod rate {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_rates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub method_id: Uuid,
        pub zone_id: Uuid,
        pub min_weight_grams: i64,
        /// `None` = không giới hạn trên.
        pub max_weight_grams: Option<i64>,
        pub max_side_mm: Option<i64>,
        pub amount_minor: i64,
        pub currency: String,
        pub step_grams: Option<i64>,
        pub step_amount_minor: Option<i64>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod pickup_point {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_pickup_points")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub method_id: Uuid,
        pub org_id: Uuid,
        pub instructions: String,
        pub prep_hours: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
