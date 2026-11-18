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
