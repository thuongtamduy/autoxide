//! Migration tạo bảng luồng thay đổi `<module>_changes` của một module.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema;

/// Migration tạo bảng `<module>_changes` (bước 69 P2). Module khai `sync_resources` thêm nó vào
/// CUỐI danh sách migration của mình:
///
/// ```ignore
/// autoxide_kernel::sync::changes_migration("m0007_create_changes", "notes"),
/// ```
pub fn changes_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(ChangesMigration { name, module })
}

struct ChangesMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for ChangesMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for ChangesMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    // UUIDv7 sinh trong ứng dụng: cũng là con trỏ của client.
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("resource", 64))
                    .col(schema::uuid("record_id"))
                    // Nút và người sở hữu lúc đổi: lọc phạm vi như danh sách thường.
                    .col(schema::uuid_null("org_id"))
                    .col(schema::uuid_null("owner_id"))
                    .col(schema::boolean("deleted"))
                    .col(schema::timestamp_with_time_zone("changed_at"))
                    .to_owned(),
            )
            .await?;
        for (suffix, columns) in [
            // Đọc luồng: khách, loại, sau con trỏ.
            ("feed_idx", vec!["tenant_id", "resource", "id"]),
            // Worker xoá dòng quá hạn (mọi khách).
            ("changed_sweep_idx", vec!["changed_at"]),
        ] {
            let mut index = Index::create();
            index.name(format!("{table}_{suffix}")).table(Alias::new(&table));
            for column in columns {
                index.col(Alias::new(column));
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = super::table_name(self.module);
        manager.drop_table(Table::drop().table(Alias::new(&table)).to_owned()).await
    }
}
