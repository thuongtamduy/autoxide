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
