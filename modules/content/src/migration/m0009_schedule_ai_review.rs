//! Cột `ai_reviewed_by` của `content_schedules`: người (đăng nhập, không phải API key) xác nhận
//! đã duyệt nội dung do AI tạo lúc đặt lịch đăng (đề bài giao diện B4, D-266). Lịch có sẵn để
//! `NULL` (không xác nhận) — như đăng mà không gửi `ai_reviewed`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_schedule_ai_review"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("content_schedules")
                    .add_column(schema::uuid_null("ai_reviewed_by"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("content_schedules").drop_column("ai_reviewed_by").to_owned(),
            )
            .await
    }
}
