//! Migration của module `notifications`. Đã commit thì không sửa — thêm migration mới vào cuối
//! [`all`]. Mọi bảng là cấp khách hàng; index bắt đầu bằng `tenant_id`, trừ index của job dọn dẹp quét
//! cả database (`…_sweep_idx`).

use autoxide_kernel::migration::prelude::*;

pub(crate) fn all() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateNotifications),
        autoxide_kernel::event::outbox_migration("m0002_create_events", "notifications"),
        autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "notifications"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0004_tenancy_events",
            vec![autoxide_kernel::event::tenant_table("notifications")],
        ),
        autoxide_kernel::event::outbox_access_migration("k0001_event_access", "notifications"),
        autoxide_kernel::job::jobs_migration("m0005_create_jobs", "notifications"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0006_tenancy_jobs",
            vec![autoxide_kernel::job::tenant_table("notifications")],
        ),
        // Bảng `notifications_throttle`: giới hạn số tin mỗi người nhận mỗi giờ (D-116).
        autoxide_kernel::throttle::throttle_migration("m0007_create_throttle", "notifications"),
        autoxide_kernel::tenancy::migration::adopt(
            "m0008_tenancy_throttle",
            vec![autoxide_kernel::throttle::tenant_table("notifications")],
        ),
        // Mẫu khách tự soạn có nháp / xuất bản / quay lại và thương hiệu của mail (D-206, D-366).
        Box::new(CreateTemplateVersions),
    ]
}

async fn index(
    manager: &SchemaManager<'_>,
    table: &str,
    name: &str,
    columns: &[&str],
    unique: bool,
) -> Result<(), DbErr> {
    let mut index = Index::create();
    index.name(name).table(Alias::new(table));
    if unique {
        index.unique();
    }
    for column in columns {
        index.col(Alias::new(*column));
    }
    manager.create_index(index.to_owned()).await
}

struct CreateNotifications;

impl MigrationName for CreateNotifications {
    fn name(&self) -> &str {
        "m0001_create_notifications"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateNotifications {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("notifications_messages")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 100))
                    .col(schema::string_len("category", 16))
                    .col(schema::string_len("channel", 16))
                    .col(schema::uuid_null("user_id"))
                    .col(schema::string_len_null("address", 320))
                    .col(schema::string_len("purpose", 64))
                    .col(schema::string_len("locale", 16))
                    .col(schema::text("params"))
                    .col(schema::string_len_null("link", 2000))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len("skip_reason", 32))
                    .col(schema::timestamp_with_time_zone_null("not_before"))
                    .col(schema::integer("attempts"))
                    .col(schema::string_len_null("provider_reference", 200))
                    .col(schema::string_len_null("error_code", 64))
                    .col(schema::string_len_null("unsubscribe_token_hash", 64))
                    .col(schema::string_len("dedupe_key", 200))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("sent_at"))
                    .to_owned(),
            )
            .await?;
        let messages = "notifications_messages";
        index(
            manager,
            messages,
            "notifications_messages_dedupe_idx",
            &["tenant_id", "dedupe_key", "channel"],
            true,
        )
        .await?;
        index(
            manager,
            messages,
            "notifications_messages_user_idx",
            &["tenant_id", "user_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            messages,
            "notifications_messages_status_idx",
            &["tenant_id", "status", "not_before"],
            false,
        )
        .await?;
        index(
            manager,
            messages,
            "notifications_messages_unsubscribe_idx",
            &["tenant_id", "unsubscribe_token_hash"],
            false,
        )
        .await?;
        index(
            manager,
            messages,
            "notifications_messages_retention_sweep_idx",
            &["created_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("notifications_inbox")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("kind", 100))
                    .col(schema::string_len("category", 16))
                    .col(schema::string_len("title", 300))
                    .col(schema::text("body"))
                    .col(schema::string_len_null("link", 2000))
                    .col(schema::timestamp_with_time_zone_null("read_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        let inbox = "notifications_inbox";
        index(
            manager,
            inbox,
            "notifications_inbox_user_idx",
            &["tenant_id", "user_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            inbox,
            "notifications_inbox_unread_idx",
            &["tenant_id", "user_id", "read_at"],
            false,
        )
        .await?;
        index(manager, inbox, "notifications_inbox_retention_sweep_idx", &["created_at"], false)
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("notifications_preferences")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("target", 120))
                    .col(schema::string_len("channel", 16))
                    .col(schema::boolean("enabled"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(
                        Index::create()
                            .col("tenant_id")
                            .col("user_id")
                            .col("target")
                            .col("channel"),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("notifications_templates")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 100))
                    .col(schema::string_len("channel", 16))
                    .col(schema::string_len("locale", 16))
                    .col(schema::string_len("title", 300))
                    .col(schema::text("body"))
                    .col(schema::string_len("short", 500))
                    .col(schema::string_len_null("zns_template_id", 64))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(
                        Index::create().col("tenant_id").col("kind").col("channel").col("locale"),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("notifications_push_subscriptions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("endpoint", 2000))
                    .col(schema::string_len("endpoint_key", 64))
                    .col(schema::string_len("p256dh", 200))
                    .col(schema::text("auth_secret"))
                    .col(schema::string_len_null("user_agent", 300))
                    .col(schema::integer("failures"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("last_used_at"))
                    .to_owned(),
            )
            .await?;
        let subs = "notifications_push_subscriptions";
        index(
            manager,
            subs,
            "notifications_push_subscriptions_endpoint_idx",
            &["tenant_id", "endpoint_key"],
            true,
        )
        .await?;
        index(
            manager,
            subs,
            "notifications_push_subscriptions_user_idx",
            &["tenant_id", "user_id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("notifications_device_tokens")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("platform", 16))
                    .col(schema::text("token"))
                    .col(schema::string_len("token_key", 64))
                    .col(schema::string_len("app_id", 200))
                    .col(schema::string_len_null("locale", 16))
                    .col(schema::integer("failures"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("last_used_at"))
                    .to_owned(),
            )
            .await?;
        let devices = "notifications_device_tokens";
        index(
            manager,
            devices,
            "notifications_device_tokens_token_idx",
            &["tenant_id", "token_key"],
            true,
        )
        .await?;
        index(
            manager,
            devices,
            "notifications_device_tokens_user_idx",
            &["tenant_id", "user_id"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("notifications_invitations")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("channel", 16))
                    .col(schema::string_len("subject_key", 64))
                    .col(schema::timestamp_with_time_zone("sent_at"))
                    .primary_key(Index::create().col("tenant_id").col("channel").col("subject_key"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "notifications_invitations",
            "notifications_device_tokens",
            "notifications_push_subscriptions",
            "notifications_templates",
            "notifications_preferences",
            "notifications_inbox",
            "notifications_messages",
        ] {
            manager.drop_table(Table::drop().table(Alias::new(table)).to_owned()).await?;
        }
        Ok(())
    }
}

/// Mẫu khách tự soạn (cú pháp của bộ dựng mẫu kernel): bản nháp, các bản đã xuất bản (không đổi
/// sau khi ghi), bản đang dùng, và thương hiệu của mail HTML (D-206, D-366).
struct CreateTemplateVersions;

impl MigrationName for CreateTemplateVersions {
    fn name(&self) -> &str {
        "m0009_create_template_versions"
    }
}

/// Cột nội dung chung của bản nháp và bản đã xuất bản.
fn template_content(table: &mut TableCreateStatement) -> &mut TableCreateStatement {
    table
        .col(schema::string_len("subject", 300))
        .col(schema::text("body_text"))
        .col(schema::text_null("body_html"))
        .col(schema::string_len("short", 500))
        .col(schema::string_len_null("zns_template_id", 64))
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTemplateVersions {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut drafts = Table::create();
        drafts
            .table("notifications_template_drafts")
            .col(schema::uuid("tenant_id"))
            .col(schema::string_len("kind", 100))
            .col(schema::string_len("channel", 16))
            .col(schema::string_len("locale", 16));
        template_content(&mut drafts)
            .col(schema::uuid_null("updated_by"))
            .col(schema::timestamp_with_time_zone("updated_at"))
            .primary_key(Index::create().col("tenant_id").col("kind").col("channel").col("locale"));
        manager.create_table(drafts.to_owned()).await?;

        let mut versions = Table::create();
        versions
            .table("notifications_template_versions")
            .col(schema::pk_uuid("id"))
            .col(schema::uuid("tenant_id"))
            .col(schema::string_len("kind", 100))
            .col(schema::string_len("channel", 16))
            .col(schema::string_len("locale", 16))
            .col(schema::integer("version"));
        template_content(&mut versions)
            .col(schema::uuid_null("published_by"))
            .col(schema::timestamp_with_time_zone("published_at"));
        manager.create_table(versions.to_owned()).await?;
        index(
            manager,
            "notifications_template_versions",
            "notifications_template_versions_key_idx",
            &["tenant_id", "kind", "channel", "locale", "version"],
            true,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("notifications_template_live")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("kind", 100))
                    .col(schema::string_len("channel", 16))
                    .col(schema::string_len("locale", 16))
                    .col(schema::integer("version"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(
                        Index::create().col("tenant_id").col("kind").col("channel").col("locale"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("notifications_branding")
                    .col(schema::pk_uuid("tenant_id"))
                    .col(schema::string_len_null("brand_name", 100))
                    .col(schema::string_len_null("logo_url", 2000))
                    .col(schema::string_len("primary_color", 7))
                    .col(schema::string_len("background_color", 7))
                    .col(schema::string_len_null("footer_text", 1000))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "notifications_branding",
            "notifications_template_live",
            "notifications_template_versions",
            "notifications_template_drafts",
        ] {
            manager.drop_table(Table::drop().table(Alias::new(table)).to_owned()).await?;
        }
        Ok(())
    }
}
