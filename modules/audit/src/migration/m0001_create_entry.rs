//! Bảng `audit_entry`: mỗi dòng là một event đã xảy ra. Chỉ thêm, không sửa, không xoá.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_entry"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("audit_entry")
                    // Chính ID của event (UUIDv7, sinh lúc xảy ra): sắp theo ID là sắp
                    // theo thời gian, và event giao lại không tạo dòng trùng.
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("module", 64))
                    .col(schema::string_len("action", 128))
                    .col(schema::uuid_null("actor_user_id"))
                    .col(schema::uuid_null("actor_api_key_id"))
                    .col(schema::string_len_null("subject", 200))
                    // JSON dạng chữ (chạy giống nhau trên hai database).
                    .col(schema::text_null("before"))
                    .col(schema::text_null("after"))
                    .col(schema::text_null("details"))
                    .col(schema::timestamp_with_time_zone("occurred_at"))
                    .col(schema::timestamp_with_time_zone("recorded_at"))
                    .to_owned(),
            )
            .await?;
        // Lọc theo người làm, theo bản ghi, theo loại việc — kèm ID để phân trang.
        for (name, column) in [
            ("audit_entry_actor_idx", "actor_user_id"),
            ("audit_entry_subject_idx", "subject"),
            ("audit_entry_action_idx", "action"),
        ] {
            manager
                .create_index(
                    Index::create()
                        .name(name)
                        .table("audit_entry")
                        .col(column)
                        .col("id")
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("audit_entry").to_owned()).await
    }
}
