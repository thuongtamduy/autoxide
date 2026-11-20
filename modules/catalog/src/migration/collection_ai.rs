//! m0014: cột `ai_assisted` của `catalog_collections` (bước 55): chữ của bộ sưu tập (tiêu đề, mô tả,
//! SEO) soạn từ gợi ý của AI, để giao diện gắn nhãn khi công khai (Luật Trí tuệ nhân tạo
//! 134/2025/QH15 từ 01/3/2026 — cần đối chiếu văn bản gốc; như `content`, D-362). Dòng có sẵn: `false`.

use autoxide_kernel::migration::prelude::*;

pub(super) struct AddCollectionAiAssisted;

impl MigrationName for AddCollectionAiAssisted {
    fn name(&self) -> &str {
        "m0014_collection_ai_assisted"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddCollectionAiAssisted {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("catalog_collections")
                    .add_column(schema::boolean("ai_assisted").default(false))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("catalog_collections").drop_column("ai_assisted").to_owned(),
            )
            .await
    }
}
