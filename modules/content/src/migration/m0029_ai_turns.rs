//! AI tạo và sửa trang (bước 67, D-109, D-229, D-20261122-ai-pages): mỗi yêu cầu là một lượt của lịch sử
//! trò chuyện của trang — lời yêu cầu, ai yêu cầu, kết quả (revision nháp) và revision trước đó (để hoàn
//! tác). Chạy bằng job: gọi mô hình ngoài transaction.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0029_ai_turns"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_ai_turns")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("entry_id"))
                    // `compose` (cả trang) hay `edit` (sửa tiếp bằng JSON Patch).
                    .col(schema::string_len("kind", 8))
                    .col(schema::text("instruction"))
                    // JSON: `id` các khối người dùng chọn để sửa (`edit`).
                    .col(schema::text_null("selection"))
                    // `queued`, `running`, `succeeded`, `failed`.
                    .col(schema::string_len("status", 10))
                    .col(schema::integer("tries"))
                    .col(schema::string_len_null("error_code", 64))
                    .col(schema::text_null("error_detail"))
                    // Phiên bản bản nháp lúc yêu cầu: ai sửa xen giữa thì kết quả không ghi đè.
                    .col(schema::timestamp_with_time_zone("base_version"))
                    .col(schema::uuid_null("previous_revision_id"))
                    .col(schema::uuid_null("revision_id"))
                    // Người yêu cầu (ảnh chụp quyền), để job ghi nháp với đúng quyền đó.
                    .col(schema::text("principal"))
                    .col(schema::uuid("requested_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_ai_turns_entry_idx")
                    .table("content_ai_turns")
                    .col("tenant_id")
                    .col("entry_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_ai_turns").to_owned()).await
    }
}
