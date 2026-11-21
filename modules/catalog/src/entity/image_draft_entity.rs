//! Entity của bảng m0020: lần tạo sản phẩm nháp từ ảnh bằng AI.

pub(crate) mod image_draft {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_image_drafts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        /// Nút nhận sản phẩm nháp.
        pub org_id: Uuid,
        /// JSON: ID ảnh trong `media`.
        pub media_ids: String,
        pub locale: String,
        /// Danh mục người yêu cầu chọn sẵn; `None`: AI gợi ý trong cây của khách.
        pub category_id: Option<Uuid>,
        pub tax_category: String,
        /// `queued`, `running`, `succeeded`, `failed`.
        pub status: String,
        pub product_id: Option<Uuid>,
        pub error_code: Option<String>,
        /// Mô hình đã trả lời.
        pub model: Option<String>,
        pub created_by: Option<Uuid>,
        pub created_at: DateTimeUtc,
        pub finished_at: Option<DateTimeUtc>,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
