//! Migration của module `address`, theo thứ tự áp dụng. Đã commit thì không sửa — thêm migration
//! mới vào cuối [`all`].
//!
//! `address_dataset`, `address_unit` là bảng **mặt điều khiển** (D-253): danh mục hành chính dùng
//! chung mọi khách, không có `tenant_id`, chỉ có dữ liệu ở database điều khiển.

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateAddress),
        // Bảng `address_events` (transactional outbox): event `address.dataset_imported`.
        autoxide_kernel::event::outbox_migration("m0002_create_events", "address"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "address"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_helpers",
            vec![autoxide_kernel::event::tenant_table("address")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "address"),
    ]
}

struct CreateAddress;

impl MigrationName for CreateAddress {
    fn name(&self) -> &str {
        "m0001_create_address"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateAddress {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("address_dataset")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("country", 2))
                    .col(schema::string_len("effective_from", 10))
                    .col(schema::string_len("title", 300))
                    .col(schema::text("source_url"))
                    .col(schema::string_len("retrieved_on", 10))
                    .col(schema::string_len("sha256", 64).unique_key())
                    .col(schema::integer("province_count"))
                    .col(schema::integer("commune_count"))
                    .col(schema::timestamp_with_time_zone("imported_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("address_unit")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("country", 2))
                    .col(schema::integer("level"))
                    .col(schema::string_len("code", 10))
                    .col(schema::string_len("name", 200))
                    .col(schema::string_len("name_folded", 200))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len_null("parent_code", 10))
                    .col(schema::string_len("valid_from", 10))
                    .col(schema::string_len_null("valid_to", 10))
                    .col(schema::uuid("dataset_id"))
                    .to_owned(),
            )
            .await?;
        let indexes: [(&str, &[&str], bool); 3] = [
            ("address_unit_code_unique", &["country", "code", "valid_from"], true),
            ("address_unit_parent_idx", &["country", "level", "parent_code", "valid_to"], false),
            ("address_unit_name_idx", &["country", "name_folded"], false),
        ];
        for (name, columns, unique) in indexes {
            let mut index = Index::create();
            index.name(name).table("address_unit");
            if unique {
                index.unique();
            }
            for column in columns {
                index.col(*column);
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("address_unit").to_owned()).await?;
        manager.drop_table(Table::drop().table("address_dataset").to_owned()).await
    }
}
