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

pub(crate) const RECORDING_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_CONVERSATIONS_RECORDING_RETENTION_DAYS",
    "Days a call recording is kept after the call ends (1 to 3650); older recordings are deleted \
     every day.",
)
.with_default("90");

pub(crate) const CALLS_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_CONVERSATIONS_CALLS_PER_DAY",
    "Calls and call-back requests one customer may make per day (1 to 100); more are answered \
     with 429.",
)
.with_default("5");

pub(crate) const CALLBACK_DAYS_AHEAD: Setting = Setting::new(
    "AUTOXIDE_CONVERSATIONS_CALLBACK_DAYS_AHEAD",
    "How many days ahead a customer may ask to be called back (1 to 60).",
)
.with_default("7");

pub(crate) const VOICE_WEBHOOK_MAX_BYTES: Setting = Setting::new(
    "AUTOXIDE_CONVERSATIONS_VOICE_WEBHOOK_MAX_BYTES",
    "Largest notification body a voice provider may send, in bytes (1024 to 1048576).",
)
.with_default("65536");

/// Cấu hình đã đọc.
#[derive(Debug, Clone)]
pub(crate) struct ConversationsConfig {
    pub retention_days: u32,
    pub notify_after_minutes: u32,
    pub messages_per_minute: u32,
    pub max_body_chars: usize,
    pub recording_retention_days: u32,
    pub calls_per_day: u32,
    pub callback_days_ahead: u32,
    pub voice_webhook_max_bytes: usize,
}

impl Default for ConversationsConfig {
    fn default() -> Self {
        ConversationsConfig {
            retention_days: 365,
            notify_after_minutes: 5,
            messages_per_minute: 20,
            max_body_chars: 4000,
            recording_retention_days: 90,
            calls_per_day: 5,
            callback_days_ahead: 7,
            voice_webhook_max_bytes: 65536,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<ConversationsConfig, Invalid> {
    let retention = reader.get_with(&RETENTION_DAYS, |raw| parse_in_range(raw, 30u32, 3650));
    let notify = reader.get_with(&NOTIFY_AFTER_MINUTES, |raw| parse_in_range(raw, 1u32, 1440));
    let per_minute = reader.get_with(&MESSAGES_PER_MINUTE, |raw| parse_in_range(raw, 1u32, 1000));
    let body = reader.get_with(&MAX_BODY_CHARS, |raw| parse_in_range(raw, 100usize, 20_000));
    let recordings =
        reader.get_with(&RECORDING_RETENTION_DAYS, |raw| parse_in_range(raw, 1u32, 3650));
    let calls = reader.get_with(&CALLS_PER_DAY, |raw| parse_in_range(raw, 1u32, 100));
    let ahead = reader.get_with(&CALLBACK_DAYS_AHEAD, |raw| parse_in_range(raw, 1u32, 60));
    let webhook =
        reader.get_with(&VOICE_WEBHOOK_MAX_BYTES, |raw| parse_in_range(raw, 1024usize, 1_048_576));
    Ok(ConversationsConfig {
        retention_days: retention?,
        notify_after_minutes: notify?,
        messages_per_minute: per_minute?,
        max_body_chars: body?,
        recording_retention_days: recordings?,
        calls_per_day: calls?,
        callback_days_ahead: ahead?,
        voice_webhook_max_bytes: webhook?,
    })
}
