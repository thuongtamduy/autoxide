//! Sắp danh sách người đăng ký theo email hay trạng thái (`sort=`, đề bài giao diện W-058):
//! - email so sánh theo byte trên cả hai database: SQLite mặc định đã vậy (`BINARY`); Postgres cần
//!   `COLLATE "C"`, không thì sắp theo locale của máy chủ (vd bỏ qua dấu `.`, `_`) và hai database
//!   cho hai thứ tự. Phép so bằng (unique, tìm) không đổi: cả hai collation đều tất định.
//! - index `(tenant_id, status, id)` cho `sort=status`; `sort=email` dùng index unique
//!   `(tenant_id, email)` có sẵn.

use autoxide_kernel::migration::prelude::*;
use sea_orm::{ConnectionTrait, DbBackend};

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0012_subscriber_sort"
    }
}

const STATUS_INDEX: &str = "newsletter_subscribers_status_idx";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.get_database_backend() == DbBackend::Postgres {
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE newsletter_subscribers \
                     ALTER COLUMN email TYPE VARCHAR(320) COLLATE \"C\"",
                )
                .await?;
        }
        manager
            .create_index(
                Index::create()
                    .name(STATUS_INDEX)
                    .table("newsletter_subscribers")
                    .col("tenant_id")
                    .col("status")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(Index::drop().name(STATUS_INDEX).table("newsletter_subscribers").to_owned())
            .await?;
        if manager.get_database_backend() == DbBackend::Postgres {
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE newsletter_subscribers \
                     ALTER COLUMN email TYPE VARCHAR(320) COLLATE \"default\"",
                )
                .await?;
        }
        Ok(())
    }
}
