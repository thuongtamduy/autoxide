//! Mọi bảng của module `content` (D-102, D-105). Mọi bảng tên `content_…`; ID là UUIDv7 sinh
//! trong ứng dụng; JSON lưu dạng chữ. Không foreign key, kể cả trong module: service xoá
//! dòng con cùng transaction (D-105), nên hai database xử như nhau.
//!
//! "Không có cha" lưu là UUID toàn số 0 trong cột `parent_key`, không `NULL`: unique index
//! coi hai `NULL` là khác nhau (cả SQLite lẫn Postgres), nên hai trang gốc cùng slug sẽ lọt.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_content"
    }
}

/// Các trường nội dung và SEO, có ở cả bản nháp lẫn bản công khai.
fn content_columns(table: &mut TableCreateStatement) {
    table
        .col(schema::string_len("title", 300))
        .col(schema::string_len("slug", 100))
        .col(schema::text("excerpt"))
        .col(schema::text("body"))
        .col(schema::integer("body_schema_version"))
        .col(schema::text("plain_text"))
        // `search::fold` của tiêu đề, tóm tắt và văn bản thuần: để tìm không phân biệt dấu.
        .col(schema::text("search_key"))
        .col(schema::integer("word_count"))
        .col(schema::integer("reading_time_minutes"))
        .col(schema::uuid_null("featured_media_id"))
        .col(schema::text("featured_media_alt"))
        .col(schema::string_len("meta_title", 300))
        .col(schema::text("meta_description"))
        .col(schema::uuid_null("og_media_id"))
        .col(schema::text("canonical_url"))
        .col(schema::boolean("noindex"));
}

async fn index(
    manager: &SchemaManager<'_>,
    table: &'static str,
    name: &'static str,
    columns: &[&'static str],
    unique: bool,
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(table);
    for column in columns {
        index.col(*column);
    }
    if unique {
        index.unique();
    }
    manager.create_index(index.to_owned()).await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Bản nháp làm việc: luôn có, một dòng mỗi mục (mỗi bản dịch là một mục).
        let mut entries = Table::create();
        entries
            .table("content_entries")
            .col(schema::pk_uuid("id"))
            .col(schema::string_len("kind", 8))
            .col(schema::string_len("locale", 35))
            .col(schema::uuid("translation_group_id"))
            .col(schema::uuid_null("parent_id"))
            .col(schema::uuid("parent_key"))
            .col(schema::string_len("status", 16))
            .col(schema::boolean("has_unpublished_changes"));
        content_columns(&mut entries);
        entries
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"))
            // Lúc tạo revision gần nhất: sửa cách đó quá 10 phút thì tự lưu một revision.
            .col(schema::timestamp_with_time_zone("revised_at"));
        manager.create_table(entries.to_owned()).await?;
        index(
            manager,
            "content_entries",
            "content_entries_translation_unique",
            &["translation_group_id", "locale"],
            true,
        )
        .await?;
        index(
            manager,
            "content_entries",
            "content_entries_slug_unique",
            &["locale", "kind", "parent_key", "slug"],
            true,
        )
        .await?;
        index(
            manager,
            "content_entries",
            "content_entries_list_idx",
            &["kind", "locale", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "content_entries",
            "content_entries_parent_idx",
            &["parent_key", "id"],
            false,
        )
        .await?;
        index(manager, "content_entries", "content_entries_status_idx", &["status", "id"], false)
            .await?;
        index(
            manager,
            "content_entries",
            "content_entries_updated_idx",
            &["updated_at", "id"],
            false,
        )
        .await?;
        index(manager, "content_entries", "content_entries_created_by_idx", &["created_by"], false)
            .await?;

        // Bản công khai: chỉ có khi mục đang công khai.
        let mut published = Table::create();
        published
            .table("content_published")
            .col(schema::uuid("entry_id").primary_key())
            .col(schema::string_len("kind", 8))
            .col(schema::string_len("locale", 35))
            .col(schema::uuid("translation_group_id"))
            .col(schema::uuid_null("parent_id"))
            .col(schema::uuid("revision_id"))
            .col(schema::text("path"));
        content_columns(&mut published);
        published
            .col(schema::timestamp_with_time_zone("published_at"))
            .col(schema::timestamp_with_time_zone("first_published_at"))
            .col(schema::timestamp_with_time_zone("content_modified_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(published.to_owned()).await?;
        index(
            manager,
            "content_published",
            "content_published_path_unique",
            &["locale", "path"],
            true,
        )
        .await?;
        index(
            manager,
            "content_published",
            "content_published_list_idx",
            &["locale", "kind", "published_at", "entry_id"],
            false,
        )
        .await?;
        index(
            manager,
            "content_published",
            "content_published_modified_idx",
            &["content_modified_at", "entry_id"],
            false,
        )
        .await?;
        index(
            manager,
            "content_published",
            "content_published_group_idx",
            &["translation_group_id"],
            false,
        )
        .await?;
        index(manager, "content_published", "content_published_parent_idx", &["parent_id"], false)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("content_revisions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("entry_id"))
                    .col(schema::text("snapshot"))
                    .col(schema::string_len("reason", 16))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "content_revisions",
            "content_revisions_entry_idx",
            &["entry_id", "created_at", "id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("content_schedules")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("entry_id"))
                    .col(schema::string_len("action", 16))
                    .col(schema::uuid_null("revision_id"))
                    .col(schema::timestamp_with_time_zone("run_at"))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("error_code", 64))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "content_schedules",
            "content_schedules_due_idx",
            &["status", "run_at"],
            false,
        )
        .await?;
        index(
            manager,
            "content_schedules",
            "content_schedules_entry_idx",
            &["entry_id", "status"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("content_redirects")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("locale", 35))
                    .col(schema::text("from_path"))
                    .col(schema::uuid_null("to_entry_id"))
                    .col(schema::text_null("to_url"))
                    .col(schema::integer("status_code"))
                    .col(schema::string_len("source", 16))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "content_redirects",
            "content_redirects_path_unique",
            &["locale", "from_path"],
            true,
        )
        .await?;
        index(
            manager,
            "content_redirects",
            "content_redirects_target_idx",
            &["to_entry_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("content_tree_locks")
                    .col(schema::string_len("locale", 35).primary_key())
                    .col(schema::big_integer("version"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("content_terms")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("taxonomy", 16))
                    .col(schema::uuid_null("parent_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("content_term_translations")
                    .col(schema::uuid("term_id"))
                    .col(schema::string_len("locale", 35))
                    .col(schema::string_len("taxonomy", 16))
                    .col(schema::string_len("name", 200))
                    .col(schema::string_len("slug", 100))
                    .col(schema::text("description"))
                    .col(schema::string_len("meta_title", 300))
                    .col(schema::text("meta_description"))
                    .primary_key(Index::create().col("term_id").col("locale"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "content_term_translations",
            "content_term_slug_unique",
            &["taxonomy", "locale", "slug"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("content_entry_terms")
                    .col(schema::uuid("entry_id"))
                    .col(schema::uuid("term_id"))
                    .col(schema::integer("sort_order"))
                    .primary_key(Index::create().col("entry_id").col("term_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "content_entry_terms",
            "content_entry_terms_term_idx",
            &["term_id", "entry_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("content_authors")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid_null("user_id"))
                    .col(schema::string_len("display_name", 200))
                    .col(schema::string_len("slug", 100))
                    .col(schema::text("bio"))
                    .col(schema::uuid_null("avatar_media_id"))
                    .col(schema::text("website_url"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "content_authors", "content_authors_slug_unique", &["slug"], true).await?;
        index(manager, "content_authors", "content_authors_user_idx", &["user_id"], false).await?;

        manager
            .create_table(
                Table::create()
                    .table("content_entry_authors")
                    .col(schema::uuid("entry_id"))
                    .col(schema::uuid("author_id"))
                    .col(schema::integer("sort_order"))
                    .primary_key(Index::create().col("entry_id").col("author_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "content_entry_authors",
            "content_entry_authors_author_idx",
            &["author_id", "entry_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("content_menus")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("key", 64))
                    .col(schema::string_len("locale", 35))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "content_menus", "content_menus_unique", &["key", "locale"], true).await?;
        manager
            .create_table(
                Table::create()
                    .table("content_menu_items")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("menu_id"))
                    .col(schema::uuid_null("parent_id"))
                    .col(schema::integer("sort_order"))
                    .col(schema::string_len("label", 200))
                    .col(schema::uuid_null("target_entry_id"))
                    .col(schema::uuid_null("target_term_id"))
                    .col(schema::text_null("target_url"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "content_menu_items",
            "content_menu_items_menu_idx",
            &["menu_id", "sort_order"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "content_menu_items",
            "content_menus",
            "content_entry_authors",
            "content_authors",
            "content_entry_terms",
            "content_term_translations",
            "content_terms",
            "content_tree_locks",
            "content_redirects",
            "content_schedules",
            "content_revisions",
            "content_published",
            "content_entries",
        ] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
