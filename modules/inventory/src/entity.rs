//! Entity (SeaORM) của các bảng `inventory_…` — mọi bảng là cấp khách hàng (`tenant_id`). Chỉ
//! dùng trong module này.

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

pub(crate) mod level {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_levels")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// Nút kho (`warehouse` hay `store`).
        pub org_id: Uuid,
        pub variant_id: Uuid,
        pub on_hand: i64,
        pub available: i64,
        pub incoming: i64,
        pub reorder_point: Option<i64>,
        pub allow_negative: bool,
        pub average_cost_minor: i64,
        pub average_cost_currency: Option<String>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod movement {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_movements")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub variant_id: Uuid,
        pub level_id: Uuid,
        pub kind: String,
        /// Có dấu: dương là vào kho, âm là ra.
        pub quantity: i64,
        pub unit_cost_minor: Option<i64>,
        pub unit_cost_currency: Option<String>,
        pub lot_id: Option<Uuid>,
        pub reason_code: Option<String>,
        pub note: String,
        pub source_type: String,
        pub source_id: Uuid,
        pub actor_id: Option<Uuid>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod adjustment {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_adjustments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub reference: String,
        pub reference_fingerprint: String,
        /// `adjustment` hay `count`.
        pub kind: String,
        pub actor_id: Option<Uuid>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod settings {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_settings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub prefer_nearby: bool,
        pub allow_split: bool,
        pub province_code: Option<String>,
        pub pickup: bool,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod source {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_sources")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub selling_org_id: Uuid,
        pub warehouse_org_id: Uuid,
        pub priority: i32,
        pub active: bool,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod reservation {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_reservations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub order_id: Uuid,
        pub selling_org_id: Uuid,
        pub status: String,
        pub expires_at: Option<DateTimeUtc>,
        pub fingerprint: String,
        pub release_reason: Option<String>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod reservation_line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_reservation_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub reservation_id: Uuid,
        pub line_key: String,
        pub variant_id: Uuid,
        pub bundle_variant_id: Option<Uuid>,
        pub per_unit: i64,
        pub level_id: Uuid,
        pub org_id: Uuid,
        pub quantity: i64,
        pub fulfilled_quantity: i64,
        pub released_quantity: i64,
        pub backordered: bool,
        pub preordered: bool,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod fulfilment {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_fulfilments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub reservation_id: Uuid,
        pub fulfilment_key: String,
        pub fingerprint: String,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod restock {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_restocks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub reference: String,
        pub reference_fingerprint: String,
        pub order_id: Option<Uuid>,
        pub org_id: Uuid,
        pub lines: String,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod transfer {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_transfers")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub number: String,
        pub from_org_id: Uuid,
        pub to_org_id: Uuid,
        pub status: String,
        pub shipped_at: Option<DateTimeUtc>,
        pub reference: Option<String>,
        pub note: String,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod transfer_line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_transfer_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub transfer_id: Uuid,
        pub variant_id: Uuid,
        pub quantity: i64,
        pub received_quantity: i64,
        pub unit_cost_minor: Option<i64>,
        pub unit_cost_currency: Option<String>,
        /// JSON `[{"lot_code", "expires_on", "quantity"}]`: lô lấy ở kho gửi (FEFO).
        pub lots: String,
        /// JSON `["…"]`: serial đang đi đường.
        pub serials: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod count {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_counts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub number: String,
        pub org_id: Uuid,
        pub status: String,
        pub note: String,
        pub applied_at: Option<DateTimeUtc>,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod count_line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_count_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub count_id: Uuid,
        pub variant_id: Uuid,
        pub counted: i64,
        pub expected_at_count: i64,
        pub counted_at: DateTimeUtc,
        pub reason_code: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod lot {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_lots")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub level_id: Uuid,
        pub org_id: Uuid,
        pub variant_id: Uuid,
        pub lot_code: String,
        pub expires_on: Option<Date>,
        pub on_hand: i64,
        pub received_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod serial {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_serials")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub variant_id: Uuid,
        pub serial: String,
        pub org_id: Option<Uuid>,
        /// `in_stock`, `in_transit`, `sold`, `returned`, `written_off`.
        pub status: String,
        pub order_id: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod supplier {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_suppliers")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub code: String,
        pub name: String,
        pub name_key: String,
        pub tax_code: Option<String>,
        pub contact_name: Option<String>,
        pub phone: Option<String>,
        pub email: Option<String>,
        pub address: Option<String>,
        pub payment_terms_days: i32,
        pub currency: String,
        pub active: bool,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod purchase_order {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_purchase_orders")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub number: String,
        pub org_id: Uuid,
        pub supplier_id: Uuid,
        pub status: String,
        pub expected_on: Option<Date>,
        pub total_minor: i64,
        pub currency: String,
        pub note: String,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod purchase_order_line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_purchase_order_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub purchase_order_id: Uuid,
        pub variant_id: Uuid,
        pub quantity: i64,
        pub received_quantity: i64,
        pub unit_cost_minor: i64,
        pub unit_cost_currency: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod receipt {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_receipts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub number: String,
        pub org_id: Uuid,
        pub supplier_id: Option<Uuid>,
        pub purchase_order_id: Option<Uuid>,
        pub reference: String,
        pub reference_fingerprint: String,
        pub posted_at: DateTimeUtc,
        pub total_minor: i64,
        pub currency: String,
        pub created_by: Option<Uuid>,
        pub ledger_status: String,
        pub ledger_entry_id: Option<Uuid>,
        pub ledger_posted_at: Option<DateTimeUtc>,
        pub ledger_error_code: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod receipt_line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_receipt_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub receipt_id: Uuid,
        pub variant_id: Uuid,
        pub quantity: i64,
        pub unit_cost_minor: i64,
        pub unit_cost_currency: String,
        pub lot_code: Option<String>,
        pub expires_on: Option<Date>,
        pub serials: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod supplier_return {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_supplier_returns")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub number: String,
        pub org_id: Uuid,
        pub supplier_id: Uuid,
        pub receipt_id: Option<Uuid>,
        pub reference: String,
        pub reference_fingerprint: String,
        pub posted_at: DateTimeUtc,
        pub total_minor: i64,
        pub currency: String,
        pub created_by: Option<Uuid>,
        pub ledger_status: String,
        pub ledger_entry_id: Option<Uuid>,
        pub ledger_posted_at: Option<DateTimeUtc>,
        pub ledger_error_code: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod supplier_return_line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "inventory_supplier_return_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub supplier_return_id: Uuid,
        pub variant_id: Uuid,
        pub quantity: i64,
        pub unit_cost_minor: i64,
        pub unit_cost_currency: String,
        pub lot_code: Option<String>,
        pub serials: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
