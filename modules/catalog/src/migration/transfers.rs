//! m0014: lần nhập / xuất CSV (`catalog_transfers`, D-223…D-227). Trạng thái chỉ đi một chiều:
//! `queued` → `running` → `succeeded` / `succeeded_with_errors` / `rejected` / `failed`.

use autoxide_kernel::migration::prelude::*;

use super::{index, table};

pub(super) struct CreateTransfers;

impl MigrationName for CreateTransfers {
    fn name(&self) -> &str {
        "m0014_transfers"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTransfers {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut transfers = table("catalog_transfers");
        transfers
            .col(schema::string_len("kind", 16))
            .col(schema::string_len("status", 32))
            .col(schema::uuid_null("org_id"))
            .col(schema::string_len_null("file_key", 200))
            .col(schema::big_integer("total_rows"))
            .col(schema::big_integer("ok_rows"))
            // JSON: tối đa 1 000 lỗi theo dòng `{row, field, code, message}`.
            .col(schema::text("errors"))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone_null("finished_at"));
        manager.create_table(transfers).await?;
        index(
            manager,
            "catalog_transfers",
            "catalog_transfers_created_idx",
            &["tenant_id", "created_at", "id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("catalog_transfers")).to_owned()).await
    }
}
