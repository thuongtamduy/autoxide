//! Bảng yêu thích, vừa xem, nhắc giỏ bỏ dở và giới thiệu bạn của `customers` (bước 47 mục 5). Cột
//! giá mốc và mốc đã báo trên dòng yêu thích dành cho báo có hàng lại / giảm giá (D-233).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_engagement"
    }
}

const TABLES: [&str; 7] = [
    "customers_referrals",
    "customers_referral_codes",
    "customers_referral_rules",
    "customers_cart_reminders",
    "customers_reminder_rules",
    "customers_recent_views",
    "customers_wishlist_items",
];

async fn index(
    manager: &SchemaManager<'_>,
    table: &str,
    name: &str,
    columns: &[&str],
    unique: bool,
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    if unique {
        index.unique();
    }
    for column in columns {
        index.col(Alias::new(*column));
    }
    manager.create_index(index.to_owned()).await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("customers_wishlist_items")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::uuid("product_id"))
                    .col(schema::uuid_null("variant_id"))
                    // `variant_id` hay `''` (cả sản phẩm): khoá chính không vướng NULL.
                    .col(schema::string_len("variant_key", 36))
                    .col(schema::timestamp_with_time_zone("added_at"))
                    .col(schema::big_integer_null("baseline_price_minor"))
                    .col(schema::string_len_null("baseline_currency", 3))
                    .col(schema::timestamp_with_time_zone_null("baseline_at"))
                    .col(schema::timestamp_with_time_zone_null("back_in_stock_alerted_at"))
                    .col(schema::timestamp_with_time_zone_null("price_drop_alerted_at"))
                    .primary_key(
                        Index::create()
                            .col("tenant_id")
                            .col("profile_id")
                            .col("product_id")
                            .col("variant_key"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_wishlist_items",
            "customers_wishlist_items_product",
            &["tenant_id", "product_id", "variant_key"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_recent_views")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::uuid("product_id"))
                    .col(schema::timestamp_with_time_zone("viewed_at"))
                    .primary_key(
                        Index::create().col("tenant_id").col("profile_id").col("product_id"),
                    )
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_recent_views",
            "customers_recent_views_at",
            &["tenant_id", "viewed_at"],
            false,
        )
        .await?;
        index(
            manager,
            "customers_recent_views",
            "customers_recent_views_profile",
            &["tenant_id", "profile_id", "viewed_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_reminder_rules")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Các mốc trễ sau khi giỏ bị bỏ, phút, JSON (vd `[60, 1440]`).
                    .col(schema::text("delays_minutes"))
                    .col(schema::boolean("enabled"))
                    .col(schema::timestamp_with_time_zone("effective_from"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_reminder_rules",
            "customers_reminder_rules_from",
            &["tenant_id", "effective_from"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_cart_reminders")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("cart_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::integer("step"))
                    .col(schema::timestamp_with_time_zone("run_at"))
                    // `scheduled/sent/cancelled/skipped_no_consent`.
                    .col(schema::string_len("status", 24))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_cart_reminders",
            "customers_cart_reminders_step",
            &["tenant_id", "cart_id", "step"],
            true,
        )
        .await?;
        index(
            manager,
            "customers_cart_reminders",
            "customers_cart_reminders_due",
            &["tenant_id", "status", "run_at"],
            false,
        )
        .await?;
        index(
            manager,
            "customers_cart_reminders",
            "customers_cart_reminders_user",
            &["tenant_id", "user_id", "status"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_referral_rules")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::big_integer("referrer_points"))
                    .col(schema::big_integer("referee_points"))
                    .col(schema::uuid_null("referrer_promotion_id"))
                    .col(schema::uuid_null("referee_promotion_id"))
                    .col(schema::big_integer("min_first_order_minor"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::timestamp_with_time_zone("effective_from"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_referral_rules",
            "customers_referral_rules_from",
            &["tenant_id", "effective_from"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_referral_codes")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("profile_id"))
                    .col(schema::string_len("code", 16))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("profile_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_referral_codes",
            "customers_referral_codes_code",
            &["tenant_id", "code"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("customers_referrals")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("referrer_profile_id"))
                    .col(schema::uuid("referee_profile_id"))
                    // `pending/rewarded/rejected`.
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("reason", 64))
                    .col(schema::uuid_null("qualifying_order_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "customers_referrals",
            "customers_referrals_referee",
            &["tenant_id", "referee_profile_id"],
            true,
        )
        .await?;
        index(
            manager,
            "customers_referrals",
            "customers_referrals_referrer",
            &["tenant_id", "referrer_profile_id", "id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager.drop_table(Table::drop().table(Alias::new(table)).to_owned()).await?;
        }
        Ok(())
    }
}
