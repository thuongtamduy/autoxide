//! Cấu hình của module `support`, tiền tố `AUTOXIDE_SUPPORT_`. Mọi biến có mặc định chạy được
//! ngay. Chính sách kinh doanh (loại phiếu, hạn trả lời, giờ làm, tự đóng) là dữ liệu theo nút,
//! không phải biến môi trường.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const GUEST_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_SUPPORT_GUEST_PER_HOUR",
    "Tickets a guest (no account) may open per hour from one network address, email or phone \
     number (1 to 1000).",
)
.with_default("5");

pub(crate) const MAX_BODY_CHARS: Setting = Setting::new(
    "AUTOXIDE_SUPPORT_MAX_BODY_CHARS",
    "Longest message of a ticket, in characters (100 to 100000).",
)
.with_default("10000");

pub(crate) const TOKEN_DAYS_AFTER_CLOSE: Setting = Setting::new(
    "AUTOXIDE_SUPPORT_TOKEN_DAYS_AFTER_CLOSE",
    "Days a guest's ticket link keeps working after the ticket is closed (0 to 365).",
)
.with_default("30");

pub(crate) const AUTO_REPLY_FLAG_LIMIT: Setting = Setting::new(
    "AUTOXIDE_SUPPORT_AUTO_REPLY_FLAG_LIMIT",
    "A saved reply reported wrong by customers this many times in 7 days stops being used for \
     automatic answers (1 to 100).",
)
.with_default("3");

pub(crate) const GUEST_URL: Setting = Setting::new(
    "AUTOXIDE_SUPPORT_GUEST_URL",
    "Page of your website that shows one ticket to a guest, with `{id}` and `{token}` where the \
     ticket id and its access token go. Sent with the verification code; without it the guest \
     keeps the token shown when the ticket was opened.",
)
.with_example("https://www.example.com/support/tickets/{id}?token={token}");

/// Cấu hình đã đọc.
#[derive(Debug, Clone)]
pub(crate) struct SupportConfig {
    pub guest_per_hour: u32,
    pub max_body_chars: usize,
    pub token_days_after_close: i64,
    pub guest_url: Option<String>,
    pub auto_reply_flag_limit: u32,
}

impl Default for SupportConfig {
    fn default() -> Self {
        SupportConfig {
            guest_per_hour: 5,
            max_body_chars: 10_000,
            token_days_after_close: 30,
            guest_url: None,
            auto_reply_flag_limit: 3,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<SupportConfig, Invalid> {
    let guest = reader.get_with(&GUEST_PER_HOUR, |raw| parse_in_range(raw, 1u32, 1000));
    let body = reader.get_with(&MAX_BODY_CHARS, |raw| parse_in_range(raw, 100usize, 100_000));
    let days = reader.get_with(&TOKEN_DAYS_AFTER_CLOSE, |raw| parse_in_range(raw, 0i64, 365));
    let url = reader.optional_with(&GUEST_URL, guest_url);
    let flags = reader.get_with(&AUTO_REPLY_FLAG_LIMIT, |raw| parse_in_range(raw, 1u32, 100));
    Ok(SupportConfig {
        guest_per_hour: guest?,
        max_body_chars: body?,
        token_days_after_close: days?,
        guest_url: url?,
        auto_reply_flag_limit: flags?,
    })
}

fn guest_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    if (lower.starts_with("https://") || lower.starts_with("http://"))
        && url.contains("{id}")
        && url.contains("{token}")
    {
        Ok(url.to_owned())
    } else {
        Err("must be an http(s) URL with `{id}` and `{token}`".to_owned())
    }
}
