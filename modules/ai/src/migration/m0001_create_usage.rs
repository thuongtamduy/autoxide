//! Bảng `ai_usage`: token đã dùng mỗi ngày (UTC) theo từng việc, để giữ hạn mức
//! `AUTOXIDE_AI_DAILY_TOKEN_LIMIT` chung cho mọi tiến trình.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_usage"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("ai_usage")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("day", 10))
                    .col(schema::string_len("purpose", 64))
                    .col(schema::big_integer("input_tokens"))
                    .col(schema::big_integer("output_tokens"))
                    .col(schema::big_integer("requests"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Một dòng mỗi (ngày, việc): thêm trùng bị chặn, tiến trình thua thì cộng vào dòng có sẵn.
        manager
            .create_index(
                Index::create()
                    .name("ai_usage_day_purpose_unique")
                    .table("ai_usage")
                    .col("day")
                    .col("purpose")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("ai_usage").to_owned()).await
    }
}
