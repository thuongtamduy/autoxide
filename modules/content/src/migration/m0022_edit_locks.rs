//! Khoá sửa trang (bước 67, D-229): `content_edit_locks` giữ ai đang sửa một mục tới khi nào. Một
//! dòng mỗi mục — khoá chính `(tenant_id, entry_id)`; chiếm bằng `INSERT … ON CONFLICT DO UPDATE …
//! WHERE` (hết hạn hay của chính người đó), cú pháp chạy như nhau trên SQLite và Postgres. Khoá chỉ
//! để báo: `If-Match` vẫn là chốt cuối khi ghi.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0022_edit_locks"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_edit_locks")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("entry_id"))
                    .col(schema::uuid("user_id"))
                    // API key đang giữ khoá, khi người gọi dùng API key (agent khác người dùng).
                    .col(schema::uuid_null("api_key_id"))
                    .col(schema::timestamp_with_time_zone("acquired_at"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .primary_key(Index::create().col("tenant_id").col("entry_id"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_edit_locks").to_owned()).await
    }
}
