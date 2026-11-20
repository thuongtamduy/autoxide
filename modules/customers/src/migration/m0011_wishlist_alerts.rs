//! Báo khi sản phẩm yêu thích có hàng lại hay giảm giá (bước 47, D-233,
//! D-20261120-step47-wishlist-alerts): người mua tự bật, hàng đợi, lô đã gửi. Cột giá mốc / mốc đã báo
//! nằm sẵn trên `customers_wishlist_items` (m0007).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0011_wishlist_alerts"
    }
}

const TABLES: [&str; 3] = [
    "customers_wishlist_alert_batches",
    "customers_wishlist_alert_queue",
    "customers_wishlist_alert_settings",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("customers_wishlist_alert_settings")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::boolean("back_in_stock"))
                    .col(schema::boolean("price_drop"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("profile_id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("customers_wishlist_alert_queue")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::uuid("product_id"))
                    .col(schema::string_len("variant_key", 36))
                    // `back_in_stock` / `price_drop`.
                    .col(schema::string_len("kind", 16))
                    // Biến thể vừa có hàng (dòng theo sản phẩm); giá lúc xếp hàng (giảm giá).
                    .col(schema::uuid_null("variant_id"))
                    .col(schema::big_integer_null("price_minor"))
                    .col(schema::string_len_null("currency", 3))
                    .col(schema::timestamp_with_time_zone("queued_at"))
                    .primary_key(
                        Index::create()
                            .col("tenant_id")
                            .col("profile_id")
                            .col("product_id")
                            .col("variant_key")
                            .col("kind"),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("customers_wishlist_alert_queue_at")
                    .table("customers_wishlist_alert_queue")
                    .col("tenant_id")
                    .col("queued_at")
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("customers_wishlist_alert_batches")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::uuid("user_id"))
                    // Ngày theo giờ của khách (`YYYY-MM-DD`): trần tin mỗi ngày.
                    .col(schema::string_len("local_day", 10))
                    .col(schema::text("items"))
                    // `pending` / `sent`.
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("sent_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("customers_wishlist_alert_batches_day")
                    .table("customers_wishlist_alert_batches")
                    .col("tenant_id")
                    .col("profile_id")
                    .col("local_day")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("customers_wishlist_alert_batches_status")
                    .table("customers_wishlist_alert_batches")
                    .col("tenant_id")
                    .col("status")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
