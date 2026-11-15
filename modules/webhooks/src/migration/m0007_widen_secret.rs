//! Cột `secret` của `webhooks_endpoint` thành `text` (bước 33 B7, D-278): giá trị đã mã hoá
//! (`sealed:v1:…`) dài hơn 100 ký tự. SQLite không giới hạn độ dài `varchar` nên không phải làm
//! gì; Postgres đổi kiểu cột (`ALTER TABLE … ALTER COLUMN … TYPE`), giữ dữ liệu.

use autoxide_kernel::migration::prelude::*;
use sea_orm::DbBackend;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_widen_secret"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.get_database_backend() != DbBackend::Postgres {
            return Ok(());
        }
        manager
            .alter_table(
                Table::alter()
                    .table("webhooks_endpoint")
                    .modify_column(ColumnDef::new("secret").text().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.get_database_backend() != DbBackend::Postgres {
            return Ok(());
        }
        // Chỉ chạy được khi secret đã về chữ thường (upgrade `u0001` đã `down`).
        manager
            .alter_table(
                Table::alter()
                    .table("webhooks_endpoint")
                    .modify_column(ColumnDef::new("secret").string_len(100).not_null())
                    .to_owned(),
            )
            .await
    }
}
