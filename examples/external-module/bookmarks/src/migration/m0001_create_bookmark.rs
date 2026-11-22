//! Table `bookmarks_bookmark`. Every table of the module `bookmarks` is named `bookmarks_…`.
//!
//! Written with the schema builder, not SQL: the same migration runs on SQLite and PostgreSQL.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_bookmark"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("bookmarks_bookmark")
                    .col(schema::pk_uuid("id"))
                    // A tenant-level table: `tenant_id`, NOT NULL, no default.
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("owner_id"))
                    .col(schema::string_len("url", 2000))
                    .col(schema::string_len("title", 200))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        // A user keeps a link once: the database refuses the second one, also when two requests
        // arrive together. Every key and index of a tenant-level table starts with `tenant_id`.
        manager
            .create_index(
                Index::create()
                    .name("bookmarks_bookmark_owner_url_unique")
                    .table("bookmarks_bookmark")
                    .col("tenant_id")
                    .col("owner_id")
                    .col("url")
                    .unique()
                    .to_owned(),
            )
            .await?;
        // The list of one user, paged by id.
        manager
            .create_index(
                Index::create()
                    .name("bookmarks_bookmark_owner_idx")
                    .table("bookmarks_bookmark")
                    .col("tenant_id")
                    .col("owner_id")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        // The retention job sweeps the whole database by age. An index that does not start with
        // `tenant_id` says so in its name: `…_sweep_idx`.
        manager
            .create_index(
                Index::create()
                    .name("bookmarks_bookmark_created_sweep_idx")
                    .table("bookmarks_bookmark")
                    .col("created_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("bookmarks_bookmark").to_owned()).await
    }
}
