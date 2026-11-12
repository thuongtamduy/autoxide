//! Bảng của module `forms`. Mọi bảng tên `forms_…`; ID là UUIDv7 sinh trong ứng dụng; JSON lưu
//! dạng chữ (cùng kiểu trên SQLite và Postgres). Không foreign key, kể cả trong module: service
//! xoá dòng con cùng transaction, nên hai database xử như nhau.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_forms"
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

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("forms_forms")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("key", 64))
                    .col(schema::string_len("name", 200))
                    .col(schema::string_len("status", 16))
                    .col(schema::text("fields"))
                    .col(schema::integer("version"))
                    .col(schema::text("allowed_origins"))
                    .col(schema::text("success_message"))
                    .col(schema::text("notify_emails"))
                    .col(schema::string_len_null("reply_to_field", 64))
                    .col(schema::integer_null("ip_limit_per_hour"))
                    .col(schema::integer_null("email_limit_per_hour"))
                    .col(schema::integer_null("retention_days"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "forms_forms", "forms_forms_key_unique", &["key"], true).await?;
        index(manager, "forms_forms", "forms_forms_status_idx", &["status", "id"], false).await?;
        index(manager, "forms_forms", "forms_forms_created_by_idx", &["created_by"], false).await?;

        manager
            .create_table(
                Table::create()
                    .table("forms_form_versions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("form_id"))
                    .col(schema::integer("version"))
                    .col(schema::text("fields"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "forms_form_versions",
            "forms_form_versions_unique",
            &["form_id", "version"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("forms_submissions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("form_id"))
                    .col(schema::integer("form_version"))
                    .col(schema::text("data"))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("spam_reason", 255))
                    .col(schema::string_len_null("email", 320))
                    .col(schema::string_len_null("locale", 8))
                    .col(schema::text_null("page_url"))
                    .col(schema::text_null("referrer"))
                    .col(schema::text_null("user_agent"))
                    .col(schema::string_len_null("ip_fingerprint", 64))
                    .col(schema::text_null("consent"))
                    .col(schema::string_len("content_hash", 64))
                    .col(schema::text("search_key"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Hộp thư của một form (cursor theo ID, lọc trạng thái).
        index(
            manager,
            "forms_submissions",
            "forms_submissions_inbox_idx",
            &["form_id", "status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "forms_submissions",
            "forms_submissions_form_idx",
            &["form_id", "id"],
            false,
        )
        .await?;
        // Xuất / xoá theo email.
        index(manager, "forms_submissions", "forms_submissions_email_idx", &["email"], false)
            .await?;
        // Dò bài trùng.
        index(
            manager,
            "forms_submissions",
            "forms_submissions_hash_idx",
            &["form_id", "content_hash", "created_at"],
            false,
        )
        .await?;
        // Job dọn theo thời hạn.
        index(
            manager,
            "forms_submissions",
            "forms_submissions_retention_idx",
            &["status", "created_at"],
            false,
        )
        .await?;
        index(manager, "forms_submissions", "forms_submissions_created_idx", &["created_at"], false)
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["forms_submissions", "forms_form_versions", "forms_forms"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
