//! Cột cho AI agent (bước 18, D-109): mục do API key nào tạo (quyền `content.write_own`,
//! `content.publish_own`), xin duyệt, đánh dấu nội dung do AI tạo và việc một người đã duyệt.
//!
//! Mỗi lệnh `ALTER TABLE` một cột: SQLite không nhận nhiều thay đổi trong một lệnh. Cột bool có
//! mặc định `false` để dòng có sẵn hợp lệ; cột còn lại cho phép `NULL`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0004_agent_fields"
    }
}

const ENTRY_COLUMNS: [&str; 4] =
    ["created_by_api_key_id", "ai_generated", "review_requested_at", "review_requested_by"];
const PUBLISHED_COLUMNS: [&str; 3] = ["ai_generated", "ai_reviewed_by", "ai_reviewed_at"];

async fn add(
    manager: &SchemaManager<'_>,
    table: &'static str,
    column: ColumnDef,
) -> Result<(), DbErr> {
    let mut column = column;
    manager.alter_table(Table::alter().table(table).add_column(&mut column).to_owned()).await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let entries = "content_entries";
        add(manager, entries, schema::uuid_null("created_by_api_key_id")).await?;
        add(manager, entries, schema::boolean("ai_generated").default(false).to_owned()).await?;
        add(manager, entries, schema::timestamp_with_time_zone_null("review_requested_at")).await?;
        add(manager, entries, schema::uuid_null("review_requested_by")).await?;
        let published = "content_published";
        add(manager, published, schema::boolean("ai_generated").default(false).to_owned()).await?;
        add(manager, published, schema::uuid_null("ai_reviewed_by")).await?;
        add(manager, published, schema::timestamp_with_time_zone_null("ai_reviewed_at")).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, columns) in
            [("content_published", &PUBLISHED_COLUMNS[..]), ("content_entries", &ENTRY_COLUMNS[..])]
        {
            for column in columns.iter().rev() {
                manager
                    .alter_table(Table::alter().table(table).drop_column(*column).to_owned())
                    .await?;
            }
        }
        Ok(())
    }
}
