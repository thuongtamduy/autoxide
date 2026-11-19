//! Bảng gọi điện trong chat (bước 61, D-232): đường dây của nút, yêu cầu gọi lại, cuộc gọi, event
//! webhook của nhà cung cấp. Mọi bảng có `tenant_id` đứng đầu khoá và index; trạng thái là chuỗi.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_voice"
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
        let l = "conversations_voice_lines";
        manager
            .create_table(
                Table::create()
                    .table(l)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("provider", 32))
                    // Số công khai của cửa hàng, E.164.
                    .col(schema::string_len("hotline", 32))
                    // Băm của token trong đường dẫn webhook (token chỉ hiện một lần).
                    .col(schema::string_len("line_token_hash", 64))
                    // Ghi âm: tắt mặc định.
                    .col(schema::boolean("recording_enabled"))
                    // JSON giờ nhận gọi lại theo thứ (giờ của nút), mặc định 08:00–20:00 mỗi ngày.
                    .col(schema::text("callback_hours"))
                    // Phút lệch UTC của giờ nhận gọi lại.
                    .col(schema::integer("utc_offset_minutes"))
                    .col(schema::boolean("active"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, l, "conversations_voice_lines_org_unique", &["tenant_id", "org_id"], true)
            .await?;
        index(
            manager,
            l,
            "conversations_voice_lines_token_idx",
            &["tenant_id", "line_token_hash"],
            false,
        )
        .await?;

        let c = "conversations_callback_requests";
        manager
            .create_table(
                Table::create()
                    .table(c)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("thread_id"))
                    .col(schema::uuid_null("requester_user_id"))
                    .col(schema::string_len("client_id", 100))
                    // Số người mua: chỉ dạng mã hoá theo khách (`seal`) và dạng che.
                    .col(schema::text_null("phone_sealed"))
                    .col(schema::string_len("phone_masked", 32))
                    // `account` / `otp`.
                    .col(schema::string_len("phone_source", 16))
                    .col(schema::timestamp_with_time_zone("window_start"))
                    .col(schema::timestamp_with_time_zone("window_end"))
                    .col(schema::timestamp_with_time_zone_null("recording_ack_at"))
                    // `open` / `calling` / `done` / `cancelled` / `expired`.
                    .col(schema::string_len("status", 16))
                    // `= thread_id` khi `open` / `calling`, NULL khi không: một yêu cầu mở mỗi cuộc.
                    .col(schema::string_len_null("open_key", 64))
                    .col(schema::uuid_null("assigned_user_id"))
                    .col(schema::integer("attempts"))
                    .col(schema::uuid_null("last_call_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            c,
            "conversations_callback_requests_client_unique",
            &["tenant_id", "thread_id", "client_id"],
            true,
        )
        .await?;
        index(
            manager,
            c,
            "conversations_callback_requests_open_unique",
            &["tenant_id", "open_key"],
            true,
        )
        .await?;
        index(
            manager,
            c,
            "conversations_callback_requests_org_idx",
            &["tenant_id", "org_id", "status", "window_start"],
            false,
        )
        .await?;

        let k = "conversations_calls";
        manager
            .create_table(
                Table::create()
                    .table(k)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("thread_id"))
                    .col(schema::uuid_null("callback_id"))
                    // `inbound` / `outbound`.
                    .col(schema::string_len("direction", 16))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len("provider", 32))
                    .col(schema::string_len_null("provider_call_id", 200))
                    .col(schema::uuid_null("agent_user_id"))
                    .col(schema::uuid_null("caller_user_id"))
                    .col(schema::timestamp_with_time_zone_null("started_at"))
                    .col(schema::timestamp_with_time_zone_null("answered_at"))
                    .col(schema::timestamp_with_time_zone_null("ended_at"))
                    .col(schema::integer_null("duration_seconds"))
                    .col(schema::boolean("recording_enabled"))
                    .col(schema::timestamp_with_time_zone_null("recording_ack_at"))
                    .col(schema::string_len_null("provider_recording_id", 200))
                    // Khoá trong `Storage` của khách.
                    .col(schema::string_len_null("recording_ref", 300))
                    .col(schema::timestamp_with_time_zone_null("recording_expires_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            k,
            "conversations_calls_provider_unique",
            &["tenant_id", "provider", "provider_call_id"],
            true,
        )
        .await?;
        index(
            manager,
            k,
            "conversations_calls_thread_idx",
            &["tenant_id", "thread_id", "created_at"],
            false,
        )
        .await?;
        index(
            manager,
            k,
            "conversations_calls_org_idx",
            &["tenant_id", "org_id", "created_at", "id"],
            false,
        )
        .await?;
        index(
            manager,
            k,
            "conversations_calls_recording_idx",
            &["tenant_id", "recording_expires_at"],
            false,
        )
        .await?;

        let e = "conversations_voice_events";
        manager
            .create_table(
                Table::create()
                    .table(e)
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("provider", 32))
                    .col(schema::string_len("provider_event_id", 200))
                    .col(schema::uuid_null("call_id"))
                    // Thông báo đã đọc từ webhook (JSON chuẩn hoá) và thân nguyên văn đã kiểm chữ ký.
                    .col(schema::text("notification"))
                    .col(schema::text("body"))
                    // `received` / `processed` / `failed`.
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("received_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            e,
            "conversations_voice_events_unique",
            &["tenant_id", "provider", "provider_event_id"],
            true,
        )
        .await?;
        index(
            manager,
            e,
            "conversations_voice_events_status_idx",
            &["tenant_id", "status", "received_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in [
            "conversations_voice_events",
            "conversations_calls",
            "conversations_callback_requests",
            "conversations_voice_lines",
        ] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
