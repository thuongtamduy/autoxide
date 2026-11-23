//! Sandbox columns of `kernel_tenants` (control plane; every database has the same schema, D-253 item 2).

use crate::migration::prelude::*;

pub(crate) struct AddSandbox;

impl MigrationName for AddSandbox {
    fn name(&self) -> &str {
        "m0037_tenant_sandbox"
    }
}

const TABLE: &str = "kernel_tenants";

#[async_trait::async_trait]
impl MigrationTrait for AddSandbox {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ nhận một cột mỗi câu `ALTER TABLE`.
        let columns = [
            // Dòng cũ nhận `production`.
            schema::string_len("kind", 16).default("production").to_owned(),
            schema::uuid_null("sandbox_of"),
            schema::timestamp_with_time_zone_null("sandbox_expires_at"),
            schema::text_null("sandbox_options"),
        ];
        for column in columns {
            manager.alter_table(Table::alter().table(TABLE).add_column(column).to_owned()).await?;
        }
        manager
            .create_index(
                Index::create()
                    .name("kernel_tenants_sandbox_of_idx")
                    .table(TABLE)
                    .col("sandbox_of")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_tenants_sandbox_expiry_idx")
                    .table(TABLE)
                    .col("kind")
                    .col("sandbox_expires_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for index in ["kernel_tenants_sandbox_expiry_idx", "kernel_tenants_sandbox_of_idx"] {
            manager.drop_index(Index::drop().name(index).table(TABLE).to_owned()).await?;
        }
        for column in ["sandbox_options", "sandbox_expires_at", "sandbox_of", "kind"] {
            manager.alter_table(Table::alter().table(TABLE).drop_column(column).to_owned()).await?;
        }
        Ok(())
    }
}
