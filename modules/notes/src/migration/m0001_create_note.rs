//! Bảng `notes_note`. Mọi bảng của module `notes` tên `notes_…`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_note"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("notes_note")
                    .col(schema::pk_uuid("id"))
                    // Chủ ghi chú: chỉ giữ ID người dùng, không foreign key sang `auth`.
                    .col(schema::uuid("owner_id"))
                    .col(schema::string_len("title", 200))
                    .col(schema::text("body"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Mỗi người không có hai ghi chú cùng tên: database chặn, kể cả khi hai request
        // ghi cùng lúc. Index này cũng phục vụ tìm theo chủ.
        manager
            .create_index(
                Index::create()
                    .name("notes_note_owner_title_unique")
                    .table("notes_note")
                    .col("owner_id")
                    .col("title")
                    .unique()
                    .to_owned(),
            )
            .await?;
        // Danh sách ghi chú của một người, phân trang theo ID.
        manager
            .create_index(
                Index::create()
                    .name("notes_note_owner_idx")
                    .table("notes_note")
                    .col("owner_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("notes_note").to_owned()).await
    }
}
