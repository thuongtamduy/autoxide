//! Migration that creates the `kernel_tenant_moves` table.

use super::*;

/// The `kernel_tenant_moves` table (control plane; only the control database holds data).
pub(crate) struct CreateTenantMoves;

impl MigrationName for CreateTenantMoves {
    fn name(&self) -> &str {
        "m0011_create_tenant_moves"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTenantMoves {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("kernel_tenant_moves")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("from_placement_id"))
                    .col(schema::uuid("to_placement_id"))
                    .col(schema::boolean("keep_source"))
                    .col(schema::string_len("state", 16))
                    .col(schema::text("tables"))
                    .col(schema::text_null("error"))
                    .col(schema::string_len_null("runner", 64))
                    .col(schema::timestamp_with_time_zone_null("runner_until"))
                    .col(schema::timestamp_with_time_zone("moving_since"))
                    .col(schema::uuid_null("requested_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_tenant_moves_tenant_idx")
                    .table("kernel_tenant_moves")
                    .col("tenant_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("kernel_tenant_moves").to_owned()).await
    }
}
