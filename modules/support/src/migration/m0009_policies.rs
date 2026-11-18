//! Chính sách hỗ trợ theo nút (bước 53 mục 3) và phút làm việc đã chờ người mua của phiếu.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_policies"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("support_policies")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    // Toàn bộ chính sách là một văn bản JSON (kiểm bằng kiểu `Policy` lúc ghi).
                    .col(schema::text("body"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("org_id"))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table("support_tickets")
                    .add_column(schema::big_integer("sla_paused_minutes").default(0))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("support_tickets")
                    .drop_column("sla_paused_minutes")
                    .to_owned(),
            )
            .await?;
        manager.drop_table(Table::drop().table("support_policies").to_owned()).await
    }
}
