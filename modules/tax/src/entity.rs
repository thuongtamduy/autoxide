//! Bảng của module `tax`. Sáu bảng đầu là **mặt điều khiển** (dữ liệu luật dùng chung mọi khách,
//! D-253); `tax_profiles` là bảng cấp khách hàng.

pub(crate) mod pack {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "tax_packs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub code: String,
        pub version: i64,
        pub checked_on: String,
        pub source_note: String,
        pub content_sha256: String,
        pub imported_by: Option<Uuid>,
        pub imported_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod jurisdiction {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "tax_jurisdictions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub code: String,
        pub name: String,
        pub default_currency: String,
        pub pack_code: String,
        pub pack_version: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod category {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "tax_categories")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub jurisdiction: String,
        pub code: String,
        pub name_en: String,
        pub name_vi: String,
        pub tax_kind: String,
        pub pack_version: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod rate {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "tax_rates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub jurisdiction: String,
        pub category_code: String,
        pub tax_kind: String,
        /// `percent`, `per_unit` hay `exempt` (không chịu thuế).
        pub basis: String,
        /// Chữ thập phân chuẩn hoá (`decimal::to_column`), vd "8".
        pub rate_percent: String,
        pub per_unit_minor: Option<i64>,
        pub per_unit_currency: Option<String>,
        pub stage: i32,
        /// `YYYY-MM-DD`, gồm.
        pub effective_from: String,
        /// `YYYY-MM-DD`, không gồm; `None`: chưa hết.
        pub effective_to: Option<String>,
        pub legal_basis: String,
        pub pack_version: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod withholding {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "tax_withholding_rates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub jurisdiction: String,
        pub payer_kind: String,
        pub payee_regime: String,
        pub business_line: String,
        pub tax_kind: String,
        pub rate_percent: String,
        pub threshold_minor: Option<i64>,
        pub threshold_currency: Option<String>,
        pub effective_from: String,
        pub effective_to: Option<String>,
        pub legal_basis: String,
        pub pack_version: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod rule {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "tax_rules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub jurisdiction: String,
        pub code: String,
        /// JSON.
        pub value: String,
        pub effective_from: String,
        pub effective_to: Option<String>,
        pub legal_basis: String,
        pub pack_version: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod profile {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "tax_profiles")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub org_id: Uuid,
        pub jurisdiction: String,
        pub tax_code: Option<String>,
        /// `vat_credit`, `vat_direct`, `household`, `exempt`.
        pub regime: String,
        /// JSON: `["goods", "services"]`.
        pub business_lines: String,
        pub annual_revenue_minor: Option<i64>,
        pub annual_revenue_currency: Option<String>,
        pub legal_name: Option<String>,
        /// JSON `Address`, hay `None`.
        pub address: Option<String>,
        pub updated_by: Option<Uuid>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl autoxide_kernel::tenancy::TenantScoped for Entity {
        fn tenant_column() -> Self::Column {
            Column::TenantId
        }
    }
}
