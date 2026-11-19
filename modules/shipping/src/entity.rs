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

pub(crate) mod carrier_account {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_carrier_accounts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub carrier_code: String,
        pub name: String,
        pub credentials_sealed: String,
        pub webhook_token_hash: String,
        pub settings: String,
        pub sandbox: bool,
        pub active: bool,
        pub check_status: String,
        pub checked_at: Option<DateTimeUtc>,
        pub check_error_code: Option<String>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod shipment {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_shipments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub origin_org_id: Uuid,
        pub order_id: Uuid,
        pub order_number: String,
        pub request_key: String,
        pub request_fingerprint: String,
        pub direction: String,
        pub buyer_id: Option<Uuid>,
        pub method_id: Uuid,
        pub method_kind: String,
        pub carrier_account_id: Option<Uuid>,
        pub carrier_code: Option<String>,
        pub service_code: Option<String>,
        pub tracking_number: Option<String>,
        pub status: String,
        pub status_at: DateTimeUtc,
        pub recipient_name: String,
        pub recipient_phone: String,
        pub country: String,
        pub province_code: String,
        pub province_name: String,
        pub commune_code: String,
        pub commune_name: String,
        pub line: String,
        pub postal_code: Option<String>,
        pub declared_value_minor: i64,
        pub cod_minor: Option<i64>,
        pub currency: String,
        pub quoted_fee_minor: Option<i64>,
        pub actual_fee_minor: Option<i64>,
        pub cod_status: String,
        pub delivered_at: Option<DateTimeUtc>,
        pub attempts: i32,
        pub last_error_code: Option<String>,
        pub note: String,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod parcel {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_parcels")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub shipment_id: Uuid,
        pub position: i32,
        pub weight_grams: i64,
        pub length_mm: Option<i64>,
        pub width_mm: Option<i64>,
        pub height_mm: Option<i64>,
        pub tracking_number: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod shipment_item {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_shipment_items")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub shipment_id: Uuid,
        pub parcel_id: Option<Uuid>,
        pub order_line_key: String,
        pub variant_id: Uuid,
        pub sku: String,
        pub name: String,
        pub quantity: i64,
        pub weight_grams: Option<i64>,
        pub length_mm: Option<i64>,
        pub width_mm: Option<i64>,
        pub height_mm: Option<i64>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod tracking_event {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_tracking_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub shipment_id: Uuid,
        pub source: String,
        pub event_key: String,
        pub status: String,
        pub carrier_status: String,
        pub occurred_at: DateTimeUtc,
        pub note: String,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod webhook_inbox {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_webhook_inbox")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub carrier_account_id: Uuid,
        pub received_at: DateTimeUtc,
        pub headers: String,
        pub body: String,
        pub status: String,
        pub error: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod cod_statement {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_cod_statements")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub carrier_account_id: Uuid,
        pub statement_reference: String,
        pub content_hash: String,
        pub period_from: Date,
        pub period_to: Date,
        pub total_cod_minor: i64,
        pub total_fee_minor: i64,
        pub net_paid_minor: i64,
        pub currency: String,
        pub status: String,
        pub imported_by: Option<Uuid>,
        pub imported_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod cod_statement_line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_cod_statement_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub statement_id: Uuid,
        pub position: i32,
        pub tracking_number: String,
        pub cod_minor: i64,
        pub fee_minor: i64,
        pub shipment_id: Option<Uuid>,
        /// Cột `match`: `matched`, `amount_mismatch`, `unknown_tracking`, `not_delivered`.
        #[sea_orm(column_name = "match")]
        pub match_kind: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod cod_discrepancy {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_cod_discrepancies")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub discrepancy_key: String,
        pub statement_line_id: Option<Uuid>,
        pub shipment_id: Option<Uuid>,
        pub kind: String,
        pub expected_minor: i64,
        pub reported_minor: i64,
        pub currency: String,
        pub status: String,
        pub resolution_code: Option<String>,
        pub resolution_note: String,
        pub resolved_by: Option<Uuid>,
        pub resolved_at: Option<DateTimeUtc>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod calendar {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_calendars")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub name: String,
        pub time_zone: String,
        /// Bit 0 = thứ Hai … bit 6 = Chủ nhật.
        pub working_weekdays: i32,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod calendar_day {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "shipping_calendar_days")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub calendar_id: Uuid,
        pub day: Date,
        pub kind: String,
        pub cutoff_minute: Option<i32>,
        pub note: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
