//! Cấu hình của module `conversations`, tiền tố `AUTOXIDE_CONVERSATIONS_`. Mọi biến có mặc định
//! chạy được ngay. Chính sách theo cửa hàng (che liên hệ, bắt xác thực tài khoản) là cài đặt theo nút
//! (`settings.rs`), không phải biến môi trường.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_CONVERSATIONS_RETENTION_DAYS",
    "Days a conversation is kept after its last message (30 to 3650); older conversations are \
     deleted with their messages every day.",
)
.with_default("365");

pub(crate) const NOTIFY_AFTER_MINUTES: Setting = Setting::new(
    "AUTOXIDE_CONVERSATIONS_NOTIFY_AFTER_MINUTES",
    "Minutes a new message may stay unread before the customer or the assigned agent gets a \
     notice (1 to 1440); messages of the same window share one notice.",
)
.with_default("5");

pub(crate) const MESSAGES_PER_MINUTE: Setting = Setting::new(
    "AUTOXIDE_CONVERSATIONS_MESSAGES_PER_MINUTE",
    "Messages one person may send per minute across conversations (1 to 1000); more are answered \
     with 429.",
)
.with_default("20");

pub(crate) const MAX_BODY_CHARS: Setting = Setting::new(
    "AUTOXIDE_CONVERSATIONS_MAX_BODY_CHARS",
    "Longest chat message, in characters (100 to 20000).",
)
.with_default("4000");

/// Cấu hình đã đọc.
#[derive(Debug, Clone)]
pub(crate) struct ConversationsConfig {
    pub retention_days: u32,
    pub notify_after_minutes: u32,
    pub messages_per_minute: u32,
    pub max_body_chars: usize,
}

impl Default for ConversationsConfig {
    fn default() -> Self {
        ConversationsConfig {
            retention_days: 365,
            notify_after_minutes: 5,
            messages_per_minute: 20,
            max_body_chars: 4000,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<ConversationsConfig, Invalid> {
    let retention = reader.get_with(&RETENTION_DAYS, |raw| parse_in_range(raw, 30u32, 3650));
    let notify = reader.get_with(&NOTIFY_AFTER_MINUTES, |raw| parse_in_range(raw, 1u32, 1440));
    let per_minute = reader.get_with(&MESSAGES_PER_MINUTE, |raw| parse_in_range(raw, 1u32, 1000));
    let body = reader.get_with(&MAX_BODY_CHARS, |raw| parse_in_range(raw, 100usize, 20_000));
    Ok(ConversationsConfig {
        retention_days: retention?,
        notify_after_minutes: notify?,
        messages_per_minute: per_minute?,
        max_body_chars: body?,
    })
}
