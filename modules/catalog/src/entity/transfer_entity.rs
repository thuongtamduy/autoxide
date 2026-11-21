//! Entity của bảng m0014: lần nhập / xuất CSV.

pub(crate) mod transfer {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_transfers")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// `import` hay `export`.
        pub kind: String,
        /// `queued`, `running`, `succeeded`, `succeeded_with_errors`, `rejected`, `failed`.
        pub status: String,
        /// Nút nhận sản phẩm mới (nhập) hay nhánh được xuất; `None`: theo quyền của người tạo.
        pub org_id: Option<Uuid>,
        /// Khoá file trong `dyn Storage` của khách; `None` khi đã dọn.
        pub file_key: Option<String>,
        pub total_rows: i64,
        pub ok_rows: i64,
        /// JSON: lỗi theo dòng `[{row, field, code, message}]`, tối đa 1 000.
        pub errors: String,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub finished_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Bản nháp AI cho mô tả và SEO của một sản phẩm ở một ngôn ngữ (m0015).
pub(crate) mod suggestion {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_suggestions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub locale: String,
        /// JSON: `short_description`, `description`, `meta_title`, `meta_description`.
        pub fields: String,
        /// `pending`, `applied` hay `discarded`.
        pub status: String,
        pub model: String,
        pub created_by: Option<Uuid>,
        pub created_at: Timestamp,
        pub decided_at: Option<Timestamp>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// `alt` của một ảnh sản phẩm ở một ngôn ngữ (m0017, D-202).
pub(crate) mod media_alt {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_product_media_alts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub media_id: Uuid,
        pub locale: String,
        pub alt: String,
        pub updated_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Giá trị thuộc tính của một biến thể (m0018); `options` thành nhiều dòng như của sản phẩm.
pub(crate) mod variant_attribute {
    use autoxide_kernel::time::Date;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_variant_attributes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub variant_id: Uuid,
        pub attribute_id: Uuid,
        pub value_text: Option<String>,
        pub value_integer: Option<i64>,
        pub value_boolean: Option<bool>,
        pub value_date: Option<Date>,
        pub option_id: Option<Uuid>,
        /// JSON `{ngôn ngữ: chữ}` cho thuộc tính kiểu chữ.
        pub texts: Option<String>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
