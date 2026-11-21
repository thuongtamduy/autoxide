//! Entity của bảng mặt điều khiển `address_dataset`, `address_unit` (không có `tenant_id`).

pub(crate) mod dataset {
    use autoxide_kernel::time::Timestamp;
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "address_dataset")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub country: String,
        /// Ngày bộ dữ liệu có hiệu lực (`YYYY-MM-DD`).
        pub effective_from: String,
        /// Văn bản gốc, vd "QĐ 19/2025/QĐ-TTg".
        pub title: String,
        pub source_url: String,
        pub retrieved_on: String,
        pub sha256: String,
        pub province_count: i32,
        pub commune_count: i32,
        pub imported_at: Timestamp,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod unit {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "address_unit")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub country: String,
        /// 1: tỉnh; 2: xã.
        pub level: i32,
        /// Mã của văn bản (giữ số 0 đầu: lưu chuỗi).
        pub code: String,
        pub name: String,
        /// `search::fold(name)`: tìm không dấu.
        pub name_folded: String,
        pub kind: String,
        pub parent_code: Option<String>,
        pub valid_from: String,
        pub valid_to: Option<String>,
        pub dataset_id: Uuid,
    }

    impl ActiveModelBehavior for ActiveModel {}
}
