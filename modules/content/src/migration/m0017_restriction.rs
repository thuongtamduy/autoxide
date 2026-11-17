//! Cột `restricted_at` của `content_entries` và `content_published` (bước 37, D-363): mục bị ẩn tạm
//! vì vụ việc (báo vi phạm sở hữu trí tuệ, hàng giả…) — `content` nghe event
//! `compliance.target_restricted` / `compliance.target_restored` (hợp đồng
//! `autoxide_kernel::moderation`); trang công khai trả 451, sitemap / feed / tìm kiếm bỏ qua. Cột ở
//! cả bản nháp để đăng lại vẫn giữ ẩn. Dòng có sẵn: `NULL` (không ẩn).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0017_restriction"
    }
}

const TABLES: [&str; 2] = ["content_entries", "content_published"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager
                .alter_table(
                    Table::alter()
                        .table(table)
                        .add_column(schema::timestamp_with_time_zone_null("restricted_at"))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES.iter().rev() {
            manager
                .alter_table(Table::alter().table(*table).drop_column("restricted_at").to_owned())
                .await?;
        }
        Ok(())
    }
}
