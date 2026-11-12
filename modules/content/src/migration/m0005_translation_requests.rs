//! Dịch bằng AI (D-109): bảng yêu cầu dịch, và cột `translated_from_revision_id` của mục — bản
//! dịch được dịch từ revision nào của bản gốc (để báo bản dịch đã cũ).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_translation_requests"
    }
}

const TABLE: &str = "content_translation_requests";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("content_entries")
                    .add_column(schema::uuid_null("translated_from_revision_id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(TABLE)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("source_entry_id"))
                    .col(schema::uuid("source_revision_id"))
                    .col(schema::string_len("target_locale", 35))
                    .col(schema::uuid_null("target_entry_id"))
                    .col(schema::timestamp_with_time_zone_null("target_updated_at"))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("active_key", 80))
                    .col(schema::string_len_null("error_code", 80))
                    .col(schema::uuid_null("result_entry_id"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::uuid_null("created_by_api_key_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_translation_requests_active_unique")
                    .table(TABLE)
                    .col("active_key")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_translation_requests_source_idx")
                    .table(TABLE)
                    .col("source_entry_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(TABLE).to_owned()).await?;
        manager
            .alter_table(
                Table::alter()
                    .table("content_entries")
                    .drop_column("translated_from_revision_id")
                    .to_owned(),
            )
            .await
    }
}
