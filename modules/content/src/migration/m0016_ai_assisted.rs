//! Cột `ai_assisted` của `content_entries` và `content_published` (bước 37, Luật Trí tuệ nhân tạo
//! 134/2025/QH15 từ 01/3/2026 — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý; D-362): nội
//! dung một người viết hay sửa từ gợi ý của AI, để giao diện gắn nhãn khi công khai. Khác
//! `ai_generated` (AI viết cả bài). Dòng có sẵn: `false`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0016_ai_assisted"
    }
}

const TABLES: [&str; 2] = ["content_entries", "content_published"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Mỗi lệnh `ALTER TABLE` một cột: SQLite không nhận nhiều thay đổi trong một lệnh.
        for table in TABLES {
            manager
                .alter_table(
                    Table::alter()
                        .table(table)
                        .add_column(schema::boolean("ai_assisted").default(false))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES.iter().rev() {
            manager
                .alter_table(Table::alter().table(*table).drop_column("ai_assisted").to_owned())
                .await?;
        }
        Ok(())
    }
}
