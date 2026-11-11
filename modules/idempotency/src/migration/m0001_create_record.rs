//! Bảng `idempotency_record`: mỗi idempotency key đã dùng, kèm response lần đầu.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_record"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("idempotency_record")
                    .col(schema::pk_uuid("id"))
                    // Người gọi (băm thông tin xác thực) và key client gửi.
                    .col(schema::string_len("scope", 64))
                    .col(schema::string_len("key", 255))
                    // Băm của method, đường dẫn và thân request lần đầu.
                    .col(schema::string_len("fingerprint", 64))
                    // Response lần đầu; `NULL` khi lần đầu còn đang chạy.
                    .col(schema::integer_null("status"))
                    .col(schema::text_null("headers"))
                    .col(schema::binary_null("body"))
                    // Lần đầu giữ quyền chạy tới lúc này; quá hạn mà chưa xong thì coi như đã chết.
                    .col(schema::timestamp_with_time_zone("locked_until"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idempotency_record_key_unique")
                    .table("idempotency_record")
                    .col("scope")
                    .col("key")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idempotency_record_expires_idx")
                    .table("idempotency_record")
                    .col("expires_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("idempotency_record").to_owned()).await
    }
}
