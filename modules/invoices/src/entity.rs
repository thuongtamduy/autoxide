//! Bảng của module `invoices` — mọi bảng là cấp khách hàng (`tenant_id`).

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

pub(crate) mod series {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "invoices_series")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub kind: String,
        /// Ký hiệu, vd `1C26TAA`.
        pub symbol: String,
        pub year: i32,
        /// Nhà cung cấp hoá đơn điện tử; `None`: chứng từ nội bộ.
        pub einvoice_provider: Option<String>,
        /// `active` hay `inactive`.
        pub status: String,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod document {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "invoices_documents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub on_behalf_of_org_id: Option<Uuid>,
        pub kind: String,
        /// `adjustment` hay `replacement`.
        pub relation: Option<String>,
        pub related_document_id: Option<Uuid>,
        /// `increase` hay `decrease` (chứng từ điều chỉnh).
        pub direction: String,
        pub series_id: Option<Uuid>,
        pub number: Option<i64>,
        pub status: String,
        pub issue_date: Option<Date>,
        pub prices_include_tax: bool,
        pub currency: String,
        pub subtotal_minor: i64,
        pub tax_minor: i64,
        pub total_minor: i64,
        /// JSON `Buyer`.
        pub buyer: String,
        /// JSON ảnh chụp người bán.
        pub seller: String,
        /// JSON ảnh chụp thuế (`TaxQuote::snapshot`).
        pub tax_snapshot: String,
        pub provider_ref: Option<String>,
        pub provider_number: Option<i64>,
        pub authority_code: Option<String>,
        pub lookup_code: Option<String>,
        pub rejection_reason: Option<String>,
        pub retention_until: Option<Date>,
        pub source_module: String,
        pub source_id: Option<String>,
        pub reference: String,
        pub reference_fingerprint: String,
        pub issued_by: Option<Uuid>,
        pub issued_at: Option<DateTimeUtc>,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod line {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "invoices_lines")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub document_id: Uuid,
        pub line_no: i32,
        pub kind: String,
        pub item_ref: Option<String>,
        pub description: String,
        pub unit: Option<String>,
        /// Chữ thập phân chuẩn hoá.
        pub quantity: String,
        pub unit_price_minor: i64,
        pub discount_minor: i64,
        pub net_minor: i64,
        pub tax_category: Option<String>,
        pub rate_percent: Option<String>,
        pub tax_minor: i64,
        pub total_minor: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod mandate {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "invoices_mandates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// Bên uỷ nhiệm (người bán).
        pub principal_org_id: Uuid,
        /// Bên nhận uỷ nhiệm (vd sàn).
        pub agent_org_id: Uuid,
        /// JSON: loại chứng từ được lập thay.
        pub kinds: String,
        pub valid_from: Date,
        pub valid_to: Option<Date>,
        /// File văn bản uỷ nhiệm (module `files`).
        pub document_file_id: Option<Uuid>,
        /// `active` hay `revoked`.
        pub status: String,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub revoked_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
