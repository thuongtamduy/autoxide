//! Bảng nút và loại nút. Cấp khách hàng: `tenant_id` đứng đầu mọi khoá và index.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_units"
    }
}

async fn index(
    manager: &SchemaManager<'_>,
    table: &'static str,
    name: &'static str,
    columns: &[&'static str],
    unique: bool,
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    for column in columns {
        index.col(Alias::new(*column));
    }
    if unique {
        index.unique();
    }
    manager.create_index(index.to_owned()).await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("organizations_unit_type")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("code", 32))
                    .col(schema::string_len("name", 100))
                    .col(schema::text("description"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("code"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("organizations_unit")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid_null("parent_id"))
                    .col(schema::string_len("unit_type", 32))
                    .col(schema::string_len("name", 200))
                    .col(schema::string_len("name_folded", 200))
                    .col(schema::string_len_null("code", 64))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("status_reason", 500))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Hai NULL khác nhau trên cả hai database: nhiều nút không mã cùng tồn tại được.
        index(
            manager,
            "organizations_unit",
            "organizations_unit_code_unique",
            &["tenant_id", "code"],
            true,
        )
        .await?;
        index(
            manager,
            "organizations_unit",
            "organizations_unit_list_idx",
            &["tenant_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "organizations_unit",
            "organizations_unit_parent_idx",
            &["tenant_id", "parent_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "organizations_unit",
            "organizations_unit_status_idx",
            &["tenant_id", "status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "organizations_unit",
            "organizations_unit_type_idx",
            &["tenant_id", "unit_type", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "organizations_unit",
            "organizations_unit_name_idx",
            &["tenant_id", "name_folded"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("organizations_unit").to_owned()).await?;
        manager.drop_table(Table::drop().table("organizations_unit_type").to_owned()).await
    }
}
