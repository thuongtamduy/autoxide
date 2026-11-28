//! Bảng của module `comments` (đề bài F). Mọi bảng tên `comments_…`; ID là UUIDv7 sinh trong ứng
//! dụng; trạng thái lưu dạng chữ, kiểm trong ứng dụng. Không foreign key, kể cả trong module (và
//! không bao giờ tới bảng của module khác — đích chỉ là cặp `target_type`, `target_id`): service
//! xoá dòng con cùng transaction, nên hai database xử như nhau.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_comments"
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
    index.name(name).table(table);
    for column in columns {
        index.col(*column);
    }
    if unique {
        index.unique();
    }
    manager.create_index(index.to_owned()).await
}

#[autoxide_kernel::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("comments_comments")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("target_type", 100))
                    .col(schema::string_len("target_id", 100))
                    .col(schema::uuid("root_id"))
                    .col(schema::uuid_null("parent_id"))
                    .col(schema::uuid_null("reply_to_id"))
                    .col(schema::integer("depth"))
                    .col(schema::string_len("author_kind", 8))
                    .col(schema::uuid_null("user_id"))
                    .col(schema::string_len_null("author_name", 100))
                    .col(schema::string_len_null("author_email", 320))
                    .col(schema::boolean("email_verified"))
                    .col(schema::text("body"))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("hold_reason", 255))
                    .col(schema::string_len_null("edit_token_hash", 64))
                    .col(schema::timestamp_with_time_zone_null("edited_at"))
                    .col(schema::timestamp_with_time_zone_null("published_at"))
                    .col(schema::string_len_null("ip_fingerprint", 64))
                    .col(schema::string_len_null("locale", 8))
                    .col(schema::string_len("content_hash", 64))
                    .col(schema::text("search_key"))
                    .col(schema::integer("reply_count"))
                    .col(schema::integer("report_count"))
                    .col(schema::timestamp_with_time_zone_null("reports_held_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Bình luận gốc của một đích (cursor theo ID, lọc trạng thái).
        index(
            manager,
            "comments_comments",
            "comments_comments_thread_idx",
            &["target_type", "target_id", "depth", "status", "id"],
            false,
        )
        .await?;
        // Trả lời của một luồng.
        index(
            manager,
            "comments_comments",
            "comments_comments_replies_idx",
            &["root_id", "status", "id"],
            false,
        )
        .await?;
        // Hàng chờ duyệt.
        index(
            manager,
            "comments_comments",
            "comments_comments_queue_idx",
            &["status", "id"],
            false,
        )
        .await?;
        // "Đã từng được duyệt" theo tài khoản; xoá theo tài khoản.
        index(
            manager,
            "comments_comments",
            "comments_comments_user_idx",
            &["user_id", "status"],
            false,
        )
        .await?;
        // Xuất / xoá theo email.
        index(
            manager,
            "comments_comments",
            "comments_comments_email_idx",
            &["author_email"],
            false,
        )
        .await?;
        // Dò bài trùng.
        index(
            manager,
            "comments_comments",
            "comments_comments_hash_idx",
            &["target_type", "target_id", "content_hash", "created_at"],
            false,
        )
        .await?;
        // Job dọn theo thời hạn.
        index(
            manager,
            "comments_comments",
            "comments_comments_retention_idx",
            &["status", "created_at"],
            false,
        )
        .await?;
        index(
            manager,
            "comments_comments",
            "comments_comments_created_idx",
            &["created_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("comments_targets")
                    .col(schema::string_len("target_type", 100))
                    .col(schema::string_len("target_id", 100))
                    .col(schema::string_len("state", 16))
                    .col(schema::boolean("hidden"))
                    .col(schema::timestamp_with_time_zone_null("closes_at"))
                    .col(schema::integer("published_count"))
                    .col(schema::integer("pending_count"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("target_type").col("target_id"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("comments_bans")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("kind", 8))
                    .col(schema::string_len("value", 320))
                    .col(schema::string_len("reason", 500))
                    .col(schema::timestamp_with_time_zone_null("until"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "comments_bans", "comments_bans_value_idx", &["kind", "value"], false)
            .await?;
        index(manager, "comments_bans", "comments_bans_created_by_idx", &["created_by"], false)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("comments_reports")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("comment_id"))
                    .col(schema::string_len("reporter_key", 100))
                    .col(schema::uuid_null("reporter_user_id"))
                    .col(schema::string_len("reason", 16))
                    .col(schema::string_len_null("note", 500))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "comments_reports",
            "comments_reports_unique",
            &["comment_id", "reporter_key"],
            true,
        )
        .await?;
        index(
            manager,
            "comments_reports",
            "comments_reports_user_idx",
            &["reporter_user_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("comments_reply_subscriptions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("comment_id"))
                    .col(schema::string_len("email", 320))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "comments_reply_subscriptions",
            "comments_reply_subscriptions_unique",
            &["comment_id", "email"],
            true,
        )
        .await?;
        index(
            manager,
            "comments_reply_subscriptions",
            "comments_reply_subscriptions_email_idx",
            &["email"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("comments_profiles")
                    .col(schema::uuid("user_id").primary_key())
                    .col(schema::string_len("display_name", 100))
                    .col(schema::string_len("name_key", 100))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "comments_profiles", "comments_profiles_name_unique", &["name_key"], true)
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "comments_profiles",
            "comments_reply_subscriptions",
            "comments_reports",
            "comments_bans",
            "comments_targets",
            "comments_comments",
        ] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
