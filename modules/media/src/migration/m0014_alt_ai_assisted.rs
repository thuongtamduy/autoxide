//! Cột `alt_ai_assisted` của `media_item` (bước 37, Luật Trí tuệ nhân tạo 134/2025/QH15 từ 01/3/2026 —
//! cần đối chiếu văn bản gốc / người có chuyên môn pháp lý; D-362): `alt` mặc định viết từ gợi ý của AI
//! (`POST /media/{id}/alt-suggestion`) để giao diện gắn nhãn. Tệp có sẵn: `false`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0014_alt_ai_assisted"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("media_item")
                    .add_column(schema::boolean("alt_ai_assisted").default(false))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("media_item").drop_column("alt_ai_assisted").to_owned(),
            )
            .await
    }
}
