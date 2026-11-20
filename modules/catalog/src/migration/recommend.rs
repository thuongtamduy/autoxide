//! m0013: dữ liệu cho sắp bán chạy / đánh giá (D-201) và gợi ý sản phẩm (B-c14):
//!
//! - `catalog_sold_products`: một dòng mỗi (đơn hoàn tất, sản phẩm) từ `orders.order_completed`;
//!   unique theo đơn nên nhận lại event không đếm hai lần.
//! - `catalog_product_stats`: số bán trong 90 ngày và tổng điểm đánh giá của sản phẩm — nguồn của
//!   cột `best_selling` / `rating_x100` trên storefront.
//! - `catalog_co_purchases`: "thường mua cùng" — số đơn chứa cả hai sản phẩm trong 90 ngày.
//! - `catalog_embeddings`: một vector mỗi (sản phẩm, ngôn ngữ) cho "sản phẩm tương tự".
//! - `catalog_similar_products`: danh sách sản phẩm tương tự tính sẵn lúc lập index, để đọc công
//!   khai không phải duyệt mọi vector.

use autoxide_kernel::migration::prelude::*;

use super::{index, table};

pub(super) struct CreateRecommendations;

impl MigrationName for CreateRecommendations {
    fn name(&self) -> &str {
        "m0013_recommendations"
    }
}

const TABLES: [&str; 5] = [
    "catalog_sold_products",
    "catalog_product_stats",
    "catalog_co_purchases",
    "catalog_embeddings",
    "catalog_similar_products",
];

#[async_trait::async_trait]
impl MigrationTrait for CreateRecommendations {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut sold = table("catalog_sold_products");
        sold.col(schema::uuid("order_id"))
            .col(schema::uuid("product_id"))
            .col(schema::big_integer("quantity"))
            .col(schema::timestamp_with_time_zone("completed_at"));
        manager.create_table(sold).await?;
        let t = "catalog_sold_products";
        index(manager, t, "catalog_sold_order_uq", &["tenant_id", "order_id", "product_id"], true)
            .await?;
        index(
            manager,
            t,
            "catalog_sold_product_idx",
            &["tenant_id", "product_id", "completed_at"],
            false,
        )
        .await?;
        index(manager, t, "catalog_sold_completed_idx", &["tenant_id", "completed_at"], false)
            .await?;

        let mut stats = table("catalog_product_stats");
        stats
            .col(schema::uuid("product_id"))
            .col(schema::big_integer("units_sold"))
            .col(schema::big_integer("rating_count"))
            .col(schema::big_integer("rating_sum"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(stats).await?;
        index(
            manager,
            "catalog_product_stats",
            "catalog_product_stats_uq",
            &["tenant_id", "product_id"],
            true,
        )
        .await?;

        let mut pairs = table("catalog_co_purchases");
        pairs
            .col(schema::uuid("product_id"))
            .col(schema::uuid("other_product_id"))
            .col(schema::big_integer("orders_count"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(pairs).await?;
        index(
            manager,
            "catalog_co_purchases",
            "catalog_co_purchases_uq",
            &["tenant_id", "product_id", "other_product_id"],
            true,
        )
        .await?;
        index(
            manager,
            "catalog_co_purchases",
            "catalog_co_purchases_top_idx",
            &["tenant_id", "product_id", "orders_count"],
            false,
        )
        .await?;

        let mut vectors = table("catalog_embeddings");
        vectors
            .col(schema::uuid("product_id"))
            .col(schema::string_len("locale", 16))
            .col(schema::string_len("content_hash", 64))
            .col(schema::string_len("model", 120))
            .col(schema::integer("dimensions"))
            .col(schema::binary("vector"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(vectors).await?;
        index(
            manager,
            "catalog_embeddings",
            "catalog_embeddings_uq",
            &["tenant_id", "product_id", "locale"],
            true,
        )
        .await?;
        index(
            manager,
            "catalog_embeddings",
            "catalog_embeddings_locale_idx",
            &["tenant_id", "locale", "model"],
            false,
        )
        .await?;

        let mut similar = table("catalog_similar_products");
        similar
            .col(schema::uuid("product_id"))
            .col(schema::string_len("locale", 16))
            .col(schema::uuid("other_product_id"))
            .col(schema::integer("score"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(similar).await?;
        index(
            manager,
            "catalog_similar_products",
            "catalog_similar_uq",
            &["tenant_id", "product_id", "locale", "other_product_id"],
            true,
        )
        .await?;
        index(
            manager,
            "catalog_similar_products",
            "catalog_similar_top_idx",
            &["tenant_id", "product_id", "locale", "score"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in TABLES.iter().rev() {
            manager.drop_table(Table::drop().table(Alias::new(*name)).to_owned()).await?;
        }
        Ok(())
    }
}
