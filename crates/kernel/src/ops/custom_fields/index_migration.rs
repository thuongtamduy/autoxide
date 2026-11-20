//! Migration tạo bảng index lọc trường tự định nghĩa của một module (tách khỏi `custom_fields.rs`
//! cho file không dài thêm — D-20261118-file-length).

use crate::migration::prelude::*;

/// Migration tạo bảng index lọc `<module>_custom_index` của module `module` (như `outbox_migration`).
pub fn index_migration(name: &'static str, module: &'static str) -> Box<dyn MigrationTrait> {
    Box::new(IndexMigration { name, module })
}

struct IndexMigration {
    name: &'static str,
    module: &'static str,
}

impl MigrationName for IndexMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for IndexMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = format!("{}_custom_index", self.module);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(&table))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("record_id"))
                    .col(schema::uuid("field_id"))
                    .col(schema::string_len_null("value_text", 400))
                    .col(schema::big_integer_null("value_int"))
                    .to_owned(),
            )
            .await?;
        for (suffix, column) in
            [("text", "value_text"), ("int", "value_int"), ("record", "record_id")]
        {
            let mut index = Index::create();
            index
                .name(format!("{table}_{suffix}_idx"))
                .table(Alias::new(&table))
                .col(Alias::new("tenant_id"));
            if column != "record_id" {
                index.col(Alias::new("field_id"));
            }
            index.col(Alias::new(column));
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop().table(Alias::new(format!("{}_custom_index", self.module))).to_owned(),
            )
            .await
    }
}
