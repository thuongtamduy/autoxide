//! Bản nháp trả lời (gợi ý AI, bước 53 mục 5): chỉ nhân viên thấy, không bao giờ tự gửi.

use autoxide_kernel::migration::prelude::*;

use super::m0001_create_support::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0011_drafts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("support_reply_drafts")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("ticket_id"))
                    .col(schema::text("body"))
                    .col(schema::boolean("ai_assisted"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("used_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "support_reply_drafts",
            "support_reply_drafts_ticket_idx",
            &["tenant_id", "ticket_id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("support_reply_drafts").to_owned()).await
    }
}
