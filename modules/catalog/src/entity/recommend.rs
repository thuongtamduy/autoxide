//! Entity của các bảng m0013: bán chạy, đánh giá, thường mua cùng, sản phẩm tương tự.

/// Một sản phẩm trong một đơn hoàn tất (`orders.order_completed`).
pub(crate) mod sold_product {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_sold_products")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub order_id: Uuid,
        pub product_id: Uuid,
        pub quantity: i64,
        pub completed_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Số bán trong 90 ngày và tổng điểm đánh giá của một sản phẩm.
pub(crate) mod product_stat {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_product_stats")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub units_sold: i64,
        pub rating_count: i64,
        /// Tổng số sao của mọi đánh giá đang hiện (1–5 mỗi đánh giá).
        pub rating_sum: i64,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// "Thường mua cùng": số đơn chứa cả `product_id` và `other_product_id` trong 90 ngày.
pub(crate) mod co_purchase {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_co_purchases")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub other_product_id: Uuid,
        pub orders_count: i64,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Vector ý nghĩa của một sản phẩm ở một ngôn ngữ.
pub(crate) mod embedding {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_embeddings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub locale: String,
        /// SHA-256 (hex) của mô hình + chữ: không đổi thì không tính lại.
        pub content_hash: String,
        pub model: String,
        pub dimensions: i32,
        /// `f32` little-endian nối nhau (`autoxide_kernel::ai::vector::encode`), đã chuẩn hoá.
        pub vector: Vec<u8>,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}

/// Một sản phẩm tương tự của `product_id` ở ngôn ngữ `locale`, tính sẵn lúc lập index.
pub(crate) mod similar_product {
    use sea_orm::entity::prelude::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "catalog_similar_products")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub tenant_id: Uuid,
        pub product_id: Uuid,
        pub locale: String,
        pub other_product_id: Uuid,
        /// Cosine × 10 000 (số nguyên để sắp như nhau trên hai database).
        pub score: i32,
        pub updated_at: DateTimeUtc,
    }

    impl ActiveModelBehavior for ActiveModel {}
    tenant_scoped!();
}
