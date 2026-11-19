//! Bảng cuộc trò chuyện, tin nhắn, con trỏ đã đọc, chặn và báo vi phạm của module `conversations`
//! (bước 61). Mọi bảng mang `tenant_id` đứng đầu mọi index (D-185); không foreign key sang bảng của
//! module khác — chỉ giữ ID.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_conversations"
    }
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

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let t = "conversations_threads";
        manager
            .create_table(
                Table::create()
                    .table(t)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Cửa hàng / gian hàng nhận cuộc trò chuyện.
                    .col(schema::uuid("org_id"))
                    // NULL sau khi tài khoản người mua bị xoá (ẩn danh hoá).
                    .col(schema::uuid_null("customer_user_id"))
                    // Bản ghi được hỏi, dạng `<kind>:<id>` (sổ đối tượng, D-123).
                    .col(schema::string_len_null("subject", 300))
                    .col(schema::string_len("status", 16))
                    .col(schema::uuid_null("assignee_user_id"))
                    // Có giá trị khi cuộc trò chuyện còn mở: mỗi người mua một cuộc mở cho mỗi
                    // (nút, bản ghi). Unique trên cột NULL chạy trên cả hai database.
                    .col(schema::string_len_null("open_key", 400))
                    .col(schema::integer("unread_customer"))
                    .col(schema::integer("unread_staff"))
                    // Trả lời tự động (D-230): người mua bấm "gặp nhân viên", số lần liên tiếp.
                    .col(schema::boolean("auto_reply_paused"))
                    .col(schema::integer("auto_replies_in_row"))
                    .col(schema::boolean("needs_agent"))
                    // Phiếu hỗ trợ đã chuyển sang (module `support`).
                    .col(schema::uuid_null("ticket_id"))
                    // Bản nháp AI cho nhân viên và tin của người mua nó trả lời.
                    .col(schema::text_null("draft_reply"))
                    .col(schema::uuid_null("draft_for"))
                    .col(schema::string_len_null("locale", 16))
                    .col(schema::timestamp_with_time_zone("last_message_at"))
                    .col(schema::timestamp_with_time_zone_null("closed_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, t, "conversations_threads_open_unique", &["tenant_id", "open_key"], true)
            .await?;
        index(
            manager,
            t,
            "conversations_threads_org_idx",
            &["tenant_id", "org_id", "status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "conversations_threads_assignee_idx",
            &["tenant_id", "assignee_user_id", "status", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "conversations_threads_customer_idx",
            &["tenant_id", "customer_user_id", "id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "conversations_threads_last_idx",
            &["tenant_id", "last_message_at"],
            false,
        )
        .await?;

        let m = "conversations_messages";
        manager
            .create_table(
                Table::create()
                    .table(m)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("thread_id"))
                    // `customer` / `agent` / `system`.
                    .col(schema::string_len("author_kind", 16))
                    .col(schema::uuid_null("author_user_id"))
                    // Chữ thuần; rỗng khi đã thu hồi hay ẩn danh hoá.
                    .col(schema::text("body"))
                    // Mảng JSON các ID của `media`.
                    .col(schema::text("media_ids"))
                    // Mã máy gửi tự sinh: gửi lại cùng mã không thêm tin thứ hai.
                    .col(schema::string_len("client_id", 100))
                    // Cuộc gọi mà tin hệ thống tóm tắt (gọi điện, D-232).
                    .col(schema::uuid_null("call_id"))
                    .col(schema::boolean("auto_reply"))
                    .col(schema::timestamp_with_time_zone_null("recalled_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            m,
            "conversations_messages_client_unique",
            &["tenant_id", "thread_id", "client_id"],
            true,
        )
        .await?;
        index(manager, m, "conversations_messages_thread_idx", &["tenant_id", "thread_id", "id"], false)
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("conversations_reads")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("thread_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::uuid("last_read_message_id"))
                    .col(schema::timestamp_with_time_zone("read_at"))
                    .primary_key(Index::create().col("tenant_id").col("thread_id").col("user_id"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "conversations_reads",
            "conversations_reads_user_idx",
            &["tenant_id", "user_id"],
            false,
        )
        .await?;

        manager
            .create_table(
                Table::create()
                    .table("conversations_blocks")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("reason", 500))
                    .col(schema::uuid_null("blocked_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "conversations_blocks",
            "conversations_blocks_unique",
            &["tenant_id", "org_id", "user_id"],
            true,
        )
        .await?;
        index(
            manager,
            "conversations_blocks",
            "conversations_blocks_user_idx",
            &["tenant_id", "user_id"],
            false,
        )
        .await?;

        let r = "conversations_reports";
        manager
            .create_table(
                Table::create()
                    .table(r)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("thread_id"))
                    .col(schema::uuid_null("message_id"))
                    // `customer` / `agent`.
                    .col(schema::string_len("reporter_kind", 16))
                    .col(schema::uuid_null("reporter_user_id"))
                    .col(schema::string_len("reason", 1000))
                    // `open` / `resolved`.
                    .col(schema::string_len("status", 16))
                    .col(schema::uuid_null("resolved_by"))
                    .col(schema::timestamp_with_time_zone_null("resolved_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, r, "conversations_reports_org_idx", &["tenant_id", "org_id", "status", "id"], false)
            .await?;
        index(manager, r, "conversations_reports_thread_idx", &["tenant_id", "thread_id"], false).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "conversations_reports",
            "conversations_blocks",
            "conversations_reads",
            "conversations_messages",
            "conversations_threads",
        ] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
