//! Loại nội dung tự định nghĩa (bước 67, D-235, D-20261122-content-types): `content_types` (đội ngũ, dự
//! án, chi nhánh…) và cột `type_id` trên mục / bản công khai (mục của loại là `kind = custom`). Trường
//! riêng của loại là trường tự định nghĩa của kernel (đích `content.type.<key>`), không bảng riêng.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0028_content_types"
    }
}

const TABLES: [&str; 2] = ["content_entries", "content_published"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_types")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("key", 64))
                    // `{"one": {"en": …}, "many": {"en": …}}` (JSON trong `text`).
                    .col(schema::text("labels"))
                    .col(schema::string_len("icon", 32))
                    .col(schema::boolean("routable"))
                    .col(schema::string_len_null("path_prefix", 64))
                    // `none`, `rich` hay `layout`.
                    .col(schema::string_len("body", 8))
                    .col(schema::boolean("has_seo"))
                    .col(schema::boolean("listable"))
                    .col(schema::boolean("active"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_types_key_unique")
                    .table("content_types")
                    .col("tenant_id")
                    .col("key")
                    .unique()
                    .to_owned(),
            )
            .await?;
        // Đường dẫn của loại không trùng trong một website (NULL — loại không có trang — không chặn nhau).
        manager
            .create_index(
                Index::create()
                    .name("content_types_prefix_unique")
                    .table("content_types")
                    .col("tenant_id")
                    .col("org_id")
                    .col("path_prefix")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_types_list_idx")
                    .table("content_types")
                    .col("tenant_id")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        // Mỗi lệnh `ALTER TABLE` một cột: SQLite không nhận nhiều thay đổi trong một lệnh.
        for table in TABLES {
            manager
                .alter_table(
                    Table::alter().table(table).add_column(schema::uuid_null("type_id")).to_owned(),
                )
                .await?;
        }
        manager
            .create_index(
                Index::create()
                    .name("content_entries_type_idx")
                    .table("content_entries")
                    .col("tenant_id")
                    .col("type_id")
                    .col("status")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_published_type_idx")
                    .table("content_published")
                    .col("tenant_id")
                    .col("type_id")
                    .col("locale")
                    .col("entry_id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("content_published_type_idx")
                    .table("content_published")
                    .to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop().name("content_entries_type_idx").table("content_entries").to_owned(),
            )
            .await?;
        for table in TABLES.iter().rev() {
            manager
                .alter_table(Table::alter().table(*table).drop_column("type_id").to_owned())
                .await?;
        }
        manager.drop_table(Table::drop().table("content_types").to_owned()).await
    }
}
