//! Bảng của module `newsletter`. Mọi bảng tên `newsletter_…`; ID là UUIDv7 sinh trong ứng dụng;
//! JSON lưu dạng chữ (cùng kiểu trên SQLite và Postgres). Không foreign key, kể cả trong module:
//! service xoá dòng con cùng transaction, nên hai database xử như nhau.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_newsletter"
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
                    .table("newsletter_lists")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("key", 64))
                    .col(schema::text("name"))
                    .col(schema::text("description"))
                    .col(schema::boolean("double_opt_in"))
                    .col(schema::boolean("public"))
                    .col(schema::string_len("new_posts", 16))
                    .col(schema::string_len_null("new_post_locale", 8))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "newsletter_lists", "newsletter_lists_key_unique", &["key"], true).await?;
        index(
            manager,
            "newsletter_lists",
            "newsletter_lists_created_by_idx",
            &["created_by"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("newsletter_subscribers")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("email", 320))
                    .col(schema::string_len_null("name", 200))
                    .col(schema::string_len("locale", 8))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("blocked_reason", 255))
                    .col(schema::text("attributes"))
                    .col(schema::integer("soft_bounces"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Một người một dòng; tìm, xuất, xoá theo email.
        index(
            manager,
            "newsletter_subscribers",
            "newsletter_subscribers_email_unique",
            &["email"],
            true,
        )
        .await?;
        index(
            manager,
            "newsletter_subscribers",
            "newsletter_subscribers_created_idx",
            &["created_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("newsletter_subscriptions")
                    .col(schema::uuid("subscriber_id"))
                    .col(schema::uuid("list_id"))
                    .col(schema::string_len("status", 16))
                    .col(schema::text_null("consent"))
                    .col(schema::string_len_null("hold_reason", 255))
                    .col(schema::timestamp_with_time_zone("requested_at"))
                    .col(schema::timestamp_with_time_zone_null("confirmed_at"))
                    .col(schema::timestamp_with_time_zone_null("unsubscribed_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("subscriber_id").col("list_id"))
                    .to_owned(),
            )
            .await?;
        // Chụp người nhận của một danh sách theo keyset (`subscriber_id`), lọc trạng thái.
        index(
            manager,
            "newsletter_subscriptions",
            "newsletter_subscriptions_list_idx",
            &["list_id", "status", "subscriber_id"],
            false,
        )
        .await?;
        // Job dọn đăng ký chưa xác nhận.
        index(
            manager,
            "newsletter_subscriptions",
            "newsletter_subscriptions_status_idx",
            &["status", "requested_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("newsletter_campaigns")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("list_id"))
                    .col(schema::string_len("subject", 255))
                    .col(schema::string_len_null("preheader", 255))
                    .col(schema::text("body_text"))
                    .col(schema::text_null("body_html"))
                    .col(schema::string_len("locale", 8))
                    .col(schema::boolean("advertising"))
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone_null("scheduled_at"))
                    .col(schema::timestamp_with_time_zone_null("started_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .col(schema::timestamp_with_time_zone_null("prepared_at"))
                    .col(schema::integer("run"))
                    .col(schema::string_len_null("source", 32))
                    .col(schema::string_len_null("source_ref", 128))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Một bài một chiến dịch (cho mỗi danh sách), dù event "đã đăng" giao lại. Chiến dịch tạo
        // tay có `NULL` — hai database đều coi các `NULL` là khác nhau trong unique index.
        index(
            manager,
            "newsletter_campaigns",
            "newsletter_campaigns_source_unique",
            &["source", "source_ref"],
            true,
        )
        .await?;
        index(
            manager,
            "newsletter_campaigns",
            "newsletter_campaigns_status_idx",
            &["status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "newsletter_campaigns",
            "newsletter_campaigns_list_idx",
            &["list_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            "newsletter_campaigns",
            "newsletter_campaigns_created_by_idx",
            &["created_by"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("newsletter_deliveries")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("campaign_id"))
                    .col(schema::uuid("subscriber_id"))
                    .col(schema::string_len("status", 16))
                    .col(schema::integer("attempts"))
                    .col(schema::string_len_null("unsubscribe_token_hash", 64))
                    .col(schema::timestamp_with_time_zone_null("claimed_at"))
                    .col(schema::timestamp_with_time_zone_null("sent_at"))
                    .col(schema::string_len_null("error", 255))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        // Mỗi người nhận một dòng: chụp lại không sinh dòng trùng.
        index(
            manager,
            "newsletter_deliveries",
            "newsletter_deliveries_unique",
            &["campaign_id", "subscriber_id"],
            true,
        )
        .await?;
        // Link huỷ đăng ký trong mail.
        index(
            manager,
            "newsletter_deliveries",
            "newsletter_deliveries_token_unique",
            &["unsubscribe_token_hash"],
            true,
        )
        .await?;
        // Job gửi lấy các dòng `pending` của một chiến dịch; đếm tiến độ.
        index(
            manager,
            "newsletter_deliveries",
            "newsletter_deliveries_status_idx",
            &["campaign_id", "status", "id"],
            false,
        )
        .await?;
        // Giới hạn thư quảng cáo mỗi 24 giờ; xoá theo người đăng ký.
        index(
            manager,
            "newsletter_deliveries",
            "newsletter_deliveries_subscriber_idx",
            &["subscriber_id", "sent_at"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("newsletter_suppressions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("email", 320))
                    .col(schema::string_len("reason", 16))
                    .col(schema::string_len_null("note", 255))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "newsletter_suppressions",
            "newsletter_suppressions_email_unique",
            &["email"],
            true,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "newsletter_suppressions",
            "newsletter_deliveries",
            "newsletter_campaigns",
            "newsletter_subscriptions",
            "newsletter_subscribers",
            "newsletter_lists",
        ] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
