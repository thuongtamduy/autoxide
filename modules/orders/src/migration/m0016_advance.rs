//! Trả trước một phần cho hàng đặt trước (D-232): kế hoạch trả trước của đơn (`orders_advance_plans`,
//! điều kiện đã công bố chụp lúc đặt) và phần trả trước, mốc chốt của từng dòng đặt trước. Mọi index có
//! `tenant_id` đứng đầu (D-185).

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0016_advance"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ thêm được một cột mỗi câu `ALTER TABLE`.
        manager
            .alter_table(
                Table::alter()
                    .table("orders_lines")
                    .add_column(schema::big_integer_null("advance_minor"))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table("orders_lines")
                    .add_column(schema::timestamp_with_time_zone_null("preorder_commit_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("orders_advance_plans")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("order_id"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::big_integer("advance_minor"))
                    .col(schema::big_integer("upfront_minor"))
                    .col(schema::big_integer("balance_minor"))
                    // `payment_link` (cách duy nhất hiện có).
                    .col(schema::string_len("balance_method", 16))
                    .col(schema::text("terms"))
                    .col(schema::string_len("terms_sha256", 64))
                    // `awaiting_advance`, `advance_paid`, `balance_requested`, `balance_paid`,
                    // `balance_overdue`, `cancelled`.
                    .col(schema::string_len("status", 24))
                    .col(schema::integer("balance_attempt").default(0))
                    .col(schema::timestamp_with_time_zone_null("balance_requested_at"))
                    .col(schema::timestamp_with_time_zone_null("balance_due_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("order_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "orders_advance_plans",
            "orders_advance_plans_due_idx",
            &["tenant_id", "status", "balance_due_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("orders_advance_plans").to_owned()).await?;
        manager
            .alter_table(
                Table::alter().table("orders_lines").drop_column("preorder_commit_at").to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter().table("orders_lines").drop_column("advance_minor").to_owned(),
            )
            .await
    }
}
