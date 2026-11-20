//! Đánh giá sau mua (D-201, D-202, D-203, D-223): `orders_reviews`, phiếu "hữu ích"
//! (`orders_review_votes`), bản nháp tóm tắt AI (`orders_review_summaries`), thống kê sao theo sản
//! phẩm (`orders_review_stats`). Chống bom hàng COD (D-194): kết quả giao của từng đơn theo khoá
//! liên hệ (`orders_delivery_outcomes`), quy tắc có ngày hiệu lực (`orders_cod_rules`), danh sách
//! chặn có lý do và hạn (`orders_cod_blocks`). Mọi index có `tenant_id` đứng đầu (D-185).

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0014_reviews_risk"
    }
}

const TABLES: [&str; 7] = [
    "orders_cod_blocks",
    "orders_cod_rules",
    "orders_delivery_outcomes",
    "orders_review_stats",
    "orders_review_summaries",
    "orders_review_votes",
    "orders_reviews",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        reviews(manager).await?;
        risk(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}

async fn reviews(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table("orders_reviews")
                .col(schema::pk_uuid("id"))
                .col(schema::uuid("tenant_id"))
                // Nút bán của đơn.
                .col(schema::uuid("org_id"))
                .col(schema::uuid("order_id"))
                .col(schema::uuid("order_line_id"))
                .col(schema::uuid("product_id"))
                .col(schema::uuid("variant_id"))
                .col(schema::uuid("author_user_id"))
                .col(schema::integer("rating"))
                .col(schema::text("body"))
                // JSON: danh sách ID ảnh của `media` (tối đa 6).
                .col(schema::text("media_ids"))
                .col(schema::boolean("has_media"))
                // `published`, `hidden`.
                .col(schema::string_len("status", 16))
                // Mã của danh sách `orders.review_hidden` (D-223).
                .col(schema::string_len_null("hidden_reason", 64))
                .col(schema::uuid_null("hidden_by"))
                .col(schema::timestamp_with_time_zone_null("hidden_at"))
                .col(schema::text_null("reply"))
                .col(schema::uuid_null("replied_by"))
                .col(schema::timestamp_with_time_zone_null("replied_at"))
                .col(schema::integer("helpful_count"))
                .col(schema::timestamp_with_time_zone("created_at"))
                .col(schema::timestamp_with_time_zone("updated_at"))
                .to_owned(),
        )
        .await?;
    let t = "orders_reviews";
    index(manager, t, "orders_reviews_line_key", &["tenant_id", "order_line_id"], true).await?;
    index(
        manager,
        t,
        "orders_reviews_product_idx",
        &["tenant_id", "product_id", "status", "id"],
        false,
    )
    .await?;
    index(
        manager,
        t,
        "orders_reviews_helpful_idx",
        &["tenant_id", "product_id", "status", "helpful_count", "id"],
        false,
    )
    .await?;
    index(manager, t, "orders_reviews_org_idx", &["tenant_id", "org_id", "created_at"], false)
        .await?;
    index(manager, t, "orders_reviews_author_idx", &["tenant_id", "author_user_id"], false).await?;
    manager
        .create_table(
            Table::create()
                .table("orders_review_votes")
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("review_id"))
                .col(schema::uuid("user_id"))
                .col(schema::timestamp_with_time_zone("created_at"))
                .primary_key(Index::create().col("tenant_id").col("review_id").col("user_id"))
                .to_owned(),
        )
        .await?;
    index(
        manager,
        "orders_review_votes",
        "orders_review_votes_user_idx",
        &["tenant_id", "user_id"],
        false,
    )
    .await?;
    manager
        .create_table(
            Table::create()
                .table("orders_review_summaries")
                .col(schema::pk_uuid("id"))
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("product_id"))
                .col(schema::string_len("locale", 8))
                // Bản đang công khai: `<product>:<locale>`; NULL với bản khác (unique có NULL).
                .col(schema::string_len_null("approved_key", 60))
                .col(schema::text_null("body"))
                // JSON: chủ đề người mua hay nhắc.
                .col(schema::text("themes"))
                // `pending`, `draft`, `approved`, `rejected`, `superseded`, `failed`.
                .col(schema::string_len("status", 16))
                .col(schema::integer("source_review_count"))
                .col(schema::string_len_null("model", 100))
                .col(schema::uuid("requested_by"))
                .col(schema::timestamp_with_time_zone_null("generated_at"))
                .col(schema::uuid_null("reviewed_by"))
                .col(schema::timestamp_with_time_zone_null("reviewed_at"))
                .col(schema::timestamp_with_time_zone("created_at"))
                .to_owned(),
        )
        .await?;
    let t = "orders_review_summaries";
    index(manager, t, "orders_review_summaries_approved_key", &["tenant_id", "approved_key"], true)
        .await?;
    index(manager, t, "orders_review_summaries_product_idx", &["tenant_id", "product_id"], false)
        .await?;
    manager
        .create_table(
            Table::create()
                .table("orders_review_stats")
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("product_id"))
                .col(schema::big_integer("review_count"))
                .col(schema::big_integer("rating_sum"))
                .col(schema::big_integer("count_1"))
                .col(schema::big_integer("count_2"))
                .col(schema::big_integer("count_3"))
                .col(schema::big_integer("count_4"))
                .col(schema::big_integer("count_5"))
                .col(schema::big_integer("media_count"))
                .col(schema::timestamp_with_time_zone("updated_at"))
                .primary_key(Index::create().col("tenant_id").col("product_id"))
                .to_owned(),
        )
        .await
        .map(|_| ())
}

async fn risk(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table("orders_delivery_outcomes")
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("order_id"))
                // Khoá liên hệ của số người mua (`SecretKey::phone_key`) — không lưu số.
                .col(schema::string_len("buyer_key", 64))
                // `delivered`, `failed`, `refused`.
                .col(schema::string_len("outcome", 16))
                .col(schema::timestamp_with_time_zone("recorded_at"))
                .primary_key(Index::create().col("tenant_id").col("order_id"))
                .to_owned(),
        )
        .await?;
    index(
        manager,
        "orders_delivery_outcomes",
        "orders_delivery_outcomes_buyer_idx",
        &["tenant_id", "buyer_key", "outcome"],
        false,
    )
    .await?;
    manager
        .create_table(
            Table::create()
                .table("orders_cod_rules")
                .col(schema::pk_uuid("id"))
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("org_id"))
                .col(schema::integer("min_orders"))
                // Tỉ lệ giao không thành (thất bại + từ chối) tối đa, phần vạn.
                .col(schema::integer("max_fail_ratio_bp"))
                .col(schema::big_integer_null("max_order_value_minor"))
                .col(schema::string_len_null("currency", 3))
                // `require_prepayment`.
                .col(schema::string_len("action", 32))
                .col(schema::timestamp_with_time_zone("effective_from"))
                .col(schema::timestamp_with_time_zone_null("effective_to"))
                .col(schema::uuid("created_by"))
                .col(schema::timestamp_with_time_zone("created_at"))
                .to_owned(),
        )
        .await?;
    index(
        manager,
        "orders_cod_rules",
        "orders_cod_rules_org_idx",
        &["tenant_id", "org_id", "effective_from"],
        false,
    )
    .await?;
    manager
        .create_table(
            Table::create()
                .table("orders_cod_blocks")
                .col(schema::pk_uuid("id"))
                .col(schema::uuid("tenant_id"))
                .col(schema::string_len("buyer_key", 64))
                // Mã của danh sách `orders.cod_block`.
                .col(schema::string_len("reason_code", 64))
                .col(schema::string_len_null("note", 1000))
                .col(schema::timestamp_with_time_zone("expires_at"))
                .col(schema::uuid("created_by"))
                .col(schema::timestamp_with_time_zone("created_at"))
                .col(schema::uuid_null("lifted_by"))
                .col(schema::timestamp_with_time_zone_null("lifted_at"))
                .to_owned(),
        )
        .await?;
    index(
        manager,
        "orders_cod_blocks",
        "orders_cod_blocks_buyer_idx",
        &["tenant_id", "buyer_key", "expires_at"],
        false,
    )
    .await
}
