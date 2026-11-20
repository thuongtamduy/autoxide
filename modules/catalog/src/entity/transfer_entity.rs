//! Entity của bảng m0014: lần nhập / xuất CSV.

pub(crate) mod transfer {
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
        pub created_at: DateTimeUtc,
        pub finished_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Bản nháp AI cho mô tả và SEO của một sản phẩm ở một ngôn ngữ (m0015).
pub(crate) mod suggestion {
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
        pub created_at: DateTimeUtc,
        pub decided_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
