//! Index cho nguồn việc "chờ duyệt" của hộp việc (bước 51 P2): đếm mục có `review_requested_at`
//! của một khách không quét cả bảng.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0015_review_index"
    }
}

const INDEX: &str = "content_entries_review_idx";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name(INDEX)
                    .table("content_entries")
                    .col("tenant_id")
                    .col("review_requested_at")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(INDEX).table("content_entries").to_owned()).await
    }
}
