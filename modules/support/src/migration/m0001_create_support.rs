//! Bảng phiếu, tin nhắn và liên kết của module `support` (bước 53). Mọi bảng mang `tenant_id`
//! đứng đầu mọi index (D-185); không foreign key sang bảng của module khác — chỉ giữ ID.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_support"
    }
}

pub(crate) async fn index(
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

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("support_tickets")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("number", 32))
                    // Người hỏi: tài khoản, hay email / số điện thoại của khách vãng lai.
                    .col(schema::uuid_null("requester_user_id"))
                    .col(schema::string_len_null("requester_email", 320))
                    .col(schema::string_len_null("requester_phone", 32))
                    .col(schema::string_len("requester_name", 200))
                    .col(schema::string_len_null("locale", 16))
                    .col(schema::timestamp_with_time_zone_null("verified_at"))
                    // Token phiếu (chỉ bản băm) và mã xác thực một lần đang chờ (bản băm).
                    .col(schema::string_len_null("token_hash", 64))
                    .col(schema::string_len_null("otp_hash", 64))
                    .col(schema::timestamp_with_time_zone_null("otp_expires_at"))
                    .col(schema::integer("otp_attempts"))
                    .col(schema::string_len("category", 64))
                    .col(schema::string_len("priority", 16))
                    .col(schema::string_len("status", 32))
                    .col(schema::string_len_null("resolution", 32))
                    .col(schema::string_len("subject", 200))
                    .col(schema::string_len("channel", 32))
                    .col(schema::uuid_null("assignee_user_id"))
                    .col(schema::uuid_null("team_id"))
                    .col(schema::timestamp_with_time_zone_null("first_response_due_at"))
                    .col(schema::timestamp_with_time_zone_null("resolve_due_at"))
                    .col(schema::timestamp_with_time_zone_null("first_responded_at"))
                    // Đồng hồ hạn trả lời đang dừng (chờ người mua) từ lúc này.
                    .col(schema::timestamp_with_time_zone_null("sla_paused_at"))
                    .col(schema::timestamp_with_time_zone_null("solved_at"))
                    .col(schema::timestamp_with_time_zone_null("closed_at"))
                    .col(schema::boolean("breached"))
                    .col(schema::uuid_null("case_id"))
                    .col(schema::uuid_null("dispute_id"))
                    .col(schema::uuid_null("follow_up_of"))
                    .col(schema::boolean("escalated_to_platform"))
                    .col(schema::small_integer_null("satisfaction"))
                    .col(schema::string_len_null("satisfaction_note", 1000))
                    // Trả lời tự động (D-230): người mua bấm "gặp nhân viên", số lần tự động liên tiếp.
                    .col(schema::boolean("auto_reply_paused"))
                    .col(schema::integer("auto_replies_in_row"))
                    .col(schema::timestamp_with_time_zone("last_message_at"))
                    .col(schema::string_len_null("reference", 300))
                    .col(schema::string("search_key"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        let t = "support_tickets";
        index(manager, t, "support_tickets_number_unique", &["tenant_id", "number"], true).await?;
        index(manager, t, "support_tickets_reference_unique", &["tenant_id", "reference"], true)
            .await?;
        index(
            manager,
            t,
            "support_tickets_org_idx",
            &["tenant_id", "org_id", "status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "support_tickets_assignee_idx",
            &["tenant_id", "assignee_user_id", "status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "support_tickets_requester_idx",
            &["tenant_id", "requester_user_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "support_tickets_due_idx",
            &["tenant_id", "status", "resolve_due_at"],
            false,
        )
        .await?;
        index(manager, t, "support_tickets_email_idx", &["tenant_id", "requester_email"], false)
            .await?;
        index(manager, t, "support_tickets_phone_idx", &["tenant_id", "requester_phone"], false)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("support_messages")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("ticket_id"))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("author_kind", 16))
                    .col(schema::uuid_null("author_user_id"))
                    .col(schema::text("body"))
                    // Mảng JSON các ID của `media`, và người được nhắc tên trong ghi chú nội bộ.
                    .col(schema::text("media_ids"))
                    .col(schema::text("mentions"))
                    .col(schema::timestamp_with_time_zone_null("hidden_at"))
                    .col(schema::uuid_null("hidden_by"))
                    .col(schema::string_len_null("hidden_reason", 500))
                    .col(schema::boolean("ai_assisted"))
                    .col(schema::boolean("auto_reply"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "support_messages",
            "support_messages_ticket_idx",
            &["tenant_id", "ticket_id", "id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("support_links")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("ticket_id"))
                    .col(schema::string_len("target", 300))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("ticket_id").col("target"))
                    .to_owned(),
            )
            .await?;
        index(manager, "support_links", "support_links_target_idx", &["tenant_id", "target"], false)
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["support_links", "support_messages", "support_tickets"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
