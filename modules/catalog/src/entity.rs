//! Entity (SeaORM) của các bảng `catalog_…` — mọi bảng là cấp khách hàng (`tenant_id`). Chỉ dùng
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

pub(crate) mod product {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_products")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub kind: String,
        pub status: String,
        pub tax_category: String,
        pub primary_category_id: Option<Uuid>,
        pub track_inventory: bool,
        /// Cờ cũ (D-204): `true` khi biến thể đầu bán theo `backorder`.
        pub allow_backorder: bool,
        pub lot_tracked: bool,
        pub serial_tracked: bool,
        pub warranty_months: Option<i32>,
        /// Dẫn xuất: `ready`, `missing_attributes`, `missing_documents`, `restricted`.
        pub readiness: String,
        pub default_locale: String,
        pub ai_assisted: bool,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
        pub activated_at: Option<DateTimeUtc>,
        pub ever_active: bool,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod translation {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_product_translations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub locale: String,
        pub name: String,
        pub slug: String,
        pub short_description: String,
        pub description: String,
        pub meta_title: Option<String>,
        pub meta_description: Option<String>,
        pub search_key: String,
        pub name_key: String,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod option {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_options")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub position: i32,
        /// JSON `{locale: tên}`.
        pub names: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod option_value {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_option_values")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub option_id: Uuid,
        pub position: i32,
        /// JSON `{locale: nhãn}`.
        pub labels: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod variant {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_variants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub sku: String,
        pub sku_key: String,
        pub barcode: Option<String>,
        pub option_key: String,
        pub position: i32,
        pub active: bool,
        pub sale_mode: String,
        pub preorder_available_on: Option<Date>,
        pub preorder_limit: Option<i64>,
        pub preorder_ends_at: Option<DateTimeUtc>,
        /// JSON `{locale: lời nhắn}`.
        pub preorder_messages: Option<String>,
        pub weight_grams: Option<i64>,
        pub length_mm: Option<i64>,
        pub width_mm: Option<i64>,
        pub height_mm: Option<i64>,
        pub image_media_id: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod product_media {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_product_media")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub media_id: Uuid,
        pub variant_id: Option<Uuid>,
        pub position: i32,
        pub alt: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod category {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_categories")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub parent_id: Option<Uuid>,
        pub depth: i32,
        pub position: i32,
        pub visible: bool,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod category_path {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_category_paths")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub ancestor_id: Uuid,
        pub descendant_id: Uuid,
        pub depth: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod category_translation {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_category_translations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub category_id: Uuid,
        pub locale: String,
        pub name: String,
        pub slug: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod product_category {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_product_categories")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub category_id: Uuid,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod bundle_component {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_bundle_components")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub bundle_variant_id: Uuid,
        pub component_variant_id: Uuid,
        pub quantity: i64,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod price_list {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_price_lists")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub name: String,
        pub currency: String,
        pub tax_inclusive: bool,
        /// `consumer` hay `business`.
        pub audience: String,
        pub company_id: Option<Uuid>,
        pub customer_group: Option<String>,
        pub channel: Option<String>,
        pub market: Option<String>,
        pub derived_from: Option<Uuid>,
        /// Điều chỉnh của bảng suy ra theo phần vạn, có dấu (−1500 = −15%).
        pub adjust_bp: i64,
        pub adjust_minor: Option<i64>,
        pub rounding_rule_id: Option<Uuid>,
        pub priority: i32,
        pub active: bool,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod price {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_prices")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub price_list_id: Uuid,
        pub variant_id: Uuid,
        pub amount_minor: i64,
        pub amount_currency: String,
        pub compare_at_minor: Option<i64>,
        pub valid_from: DateTimeUtc,
        pub valid_to: Option<DateTimeUtc>,
        /// `manual`, `derived`, `bulk`, `import` (D-222).
        pub source: String,
        pub reference: Option<String>,
        pub reference_fingerprint: Option<String>,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod price_guard {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_price_guards")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub category_id: Option<Uuid>,
        pub currency: String,
        pub min_price_minor: Option<i64>,
        pub min_margin_bp: Option<i64>,
        /// `warn` hay `block`.
        pub action: String,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod variant_cost {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_variant_costs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub variant_id: Uuid,
        pub unit_cost_minor: i64,
        pub currency: String,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod attribute {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_attributes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub code: String,
        /// `text`, `integer`, `decimal`, `boolean`, `date`, `option`, `options`.
        pub data_type: String,
        pub scale: i32,
        pub unit: Option<String>,
        /// JSON `{locale: tên}`.
        pub names: String,
        pub filterable: bool,
        pub legal_basis: Option<String>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod attribute_option {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_attribute_options")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub attribute_id: Uuid,
        pub code: String,
        /// JSON `{locale: nhãn}`.
        pub labels: String,
        pub position: i32,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod category_attribute {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_category_attributes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub category_id: Uuid,
        pub attribute_id: Uuid,
        pub required: bool,
        pub applies_to_descendants: bool,
        pub effective_from: Date,
        pub effective_to: Option<Date>,
        pub legal_basis: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod product_attribute {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_product_attributes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub attribute_id: Uuid,
        pub value_text: Option<String>,
        /// Số nguyên, hay số thập phân nhân theo `scale` của thuộc tính.
        pub value_integer: Option<i64>,
        pub value_boolean: Option<bool>,
        pub value_date: Option<Date>,
        pub option_id: Option<Uuid>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Dữ liệu luật dùng chung mọi khách (mặt điều khiển, D-198): không có `tenant_id`, đọc ở database
/// điều khiển.
pub(crate) mod restriction_rule {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_restriction_rules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub jurisdiction: String,
        pub code: String,
        /// `prohibited` hay `conditional`.
        pub level: String,
        /// JSON `{locale: tên}`.
        pub names: String,
        /// JSON: mã loại giấy tờ phải có.
        pub required_documents: String,
        pub legal_basis: String,
        pub source_url: String,
        pub effective_from: Date,
        pub effective_to: Option<Date>,
        pub pack_version: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod category_restriction {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_category_restrictions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub category_id: Uuid,
        pub rule_code: String,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod product_document {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_product_documents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub org_id: Uuid,
        pub document_type: String,
        pub number: String,
        pub issued_by: String,
        pub issued_on: Date,
        pub expires_on: Option<Date>,
        pub file_key: Option<String>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod rounding_rule {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_rounding_rules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub name: String,
        pub step_minor: i64,
        /// `up`, `down`, `nearest`.
        pub mode: String,
        pub ending_minor: Option<i64>,
        pub created_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod price_change {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_price_changes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub price_list_id: Uuid,
        /// JSON: danh mục, sản phẩm, biến thể.
        pub filter: String,
        /// `set`, `percent`, `amount`.
        pub operation: String,
        pub value: i64,
        pub rounding_rule_id: Option<Uuid>,
        pub valid_from: DateTimeUtc,
        pub valid_to: Option<DateTimeUtc>,
        /// `draft`, `previewed`, `approved`, `applied`, `cancelled`.
        pub status: String,
        pub affected: i64,
        /// JSON của bản xem trước.
        pub preview: Option<String>,
        pub created_by: Option<Uuid>,
        pub approved_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod storefront_entry {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_storefront_entries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// Nút bán có giá riêng (gốc, hay nút có bảng giá / tuỳ biến riêng).
        pub org_id: Uuid,
        pub product_id: Uuid,
        /// Nút sở hữu sản phẩm (người bán khi là sàn).
        pub owner_org_id: Uuid,
        pub visible: bool,
        pub currency: String,
        pub min_price_minor: i64,
        pub max_price_minor: i64,
        pub compare_at_minor: Option<i64>,
        pub sale_price_minor: Option<i64>,
        pub sale_ends_at: Option<DateTimeUtc>,
        pub best_selling: i64,
        /// Điểm đánh giá × 100 (0 = chưa có).
        pub rating_x100: i32,
        pub rating_count: i64,
        pub variant_count: i32,
        pub published_at: DateTimeUtc,
        pub refreshed_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod store_override {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_store_overrides")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub org_id: Uuid,
        pub product_id: Uuid,
        pub hidden: bool,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

pub(crate) mod synonym {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_synonyms")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub position: i32,
        /// JSON danh sách từ đã bỏ dấu, chữ thường (`search::fold`).
        pub terms: String,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
