//! Bài, trang thuộc một nút của cây tổ chức (bước 34, D-208): website của cửa hàng / người bán có
//! nội dung riêng. `org_id` ở `content_entries` (nút sở hữu) và chép sang `content_published` (đọc
//! công khai lọc theo website không cần nối bảng). Dòng cũ ở nút gốc (= ID của khách). Cột NULL
//! được ở database, điền ngay ở đây và service luôn ghi (D-304 mục 6).

use autoxide_kernel::migration::prelude::*;
use sea_orm::{ConnectionTrait, Statement};

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0012_org"
    }
}

const TABLES: [(&str, &str, &str); 2] = [
    ("content_entries", "content_entries_org_idx", "id"),
    ("content_published", "content_published_org_idx", "entry_id"),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let connection = manager.get_connection();
        let backend = connection.get_database_backend();
        for (table, index, id) in TABLES {
            manager
                .alter_table(
                    Table::alter().table(table).add_column(schema::uuid_null("org_id")).to_owned(),
                )
                .await?;
            connection
                .execute_raw(Statement::from_string(
                    backend,
                    format!("UPDATE {table} SET org_id = tenant_id"),
                ))
                .await?;
            manager
                .create_index(
                    Index::create()
                        .name(index)
                        .table(table)
                        .col("tenant_id")
                        .col("org_id")
                        .col(id)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, index, _) in TABLES {
            manager.drop_index(Index::drop().name(index).table(table).to_owned()).await?;
            manager
                .alter_table(Table::alter().table(table).drop_column("org_id").to_owned())
                .await?;
        }
        Ok(())
    }
}
