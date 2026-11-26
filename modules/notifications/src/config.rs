//! Cấu hình của module `notifications`, tiền tố `AUTOXIDE_NOTIFICATIONS_`. Mọi biến có mặc định chạy
//! được ngay hoặc là tuỳ chọn: thiếu `…_UNSUBSCRIBE_URL` thì chỉ email quảng cáo không gửi được (ghi
//! `failed` với `unsubscribe_url_missing`), mọi thứ khác vẫn chạy.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_NOTIFICATIONS_RETENTION_DAYS",
    "Days the address and the filled-in values of a sent notification are kept in the send log; a \
     daily job blanks them after that (1 to 3650). The log row itself stays for statistics.",
)
.with_default("90");

pub(crate) const INBOX_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_NOTIFICATIONS_INBOX_RETENTION_DAYS",
    "Days a notification stays in the in-app inbox before a daily job deletes it (1 to 3650).",
)
.with_default("180");

pub(crate) const UNSUBSCRIBE_URL: Setting = Setting::new(
    "AUTOXIDE_NOTIFICATIONS_UNSUBSCRIBE_URL",
    "Page of your website that withdraws a consent to advertising, with `{token}` where the token \
     goes; the page sends it to `POST /api/v1/notifications/public/unsubscribe`. Every advertising \
     email links there (and carries it in `List-Unsubscribe`), so advertising emails are not sent \
     without it.",
)
.with_example("https://www.example.com/unsubscribe?token={token}");

/// Trang huỷ nhận quảng cáo theo từng khách (D-20261125-tenant-link-templates).
pub(crate) const UNSUBSCRIBE_LINK: autoxide_kernel::tenancy::TenantSetting =
    autoxide_kernel::tenancy::TenantSetting::link(
        "notifications.link.unsubscribe",
        "Page of this tenant's website that withdraws a consent to advertising, such as \
         `https://shop.example/huy-nhan-tin?token={token}`: `{token}` is filled in, and the page \
         sends it to `POST /api/v1/notifications/public/unsubscribe`. Must be on this tenant's own \
         host (a verified domain of this tenant). When unset, the server's \
         AUTOXIDE_NOTIFICATIONS_UNSUBSCRIBE_URL is used; with neither, advertising emails are not sent.",
        &["{token}"],
    );

pub(crate) const PER_RECIPIENT_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_NOTIFICATIONS_PER_RECIPIENT_PER_HOUR",
    "Transactional and advertising notifications one recipient may get per hour (1 to 10000); more \
     are skipped with reason `throttled`. Security notifications are never limited.",
)
.with_default("20");

/// Cấu hình đã đọc.
#[derive(Debug, Clone)]
pub(crate) struct NotificationsConfig {
    pub retention_days: i64,
    pub inbox_retention_days: i64,
    pub unsubscribe_url: Option<String>,
    pub per_recipient_per_hour: u32,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        NotificationsConfig {
            retention_days: 90,
            inbox_retention_days: 180,
            unsubscribe_url: None,
            per_recipient_per_hour: 20,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<NotificationsConfig, Invalid> {
    let retention = reader.get_with(&RETENTION_DAYS, |raw| parse_in_range(raw, 1i64, 3650));
    let inbox = reader.get_with(&INBOX_RETENTION_DAYS, |raw| parse_in_range(raw, 1i64, 3650));
    let url = reader.optional_with(&UNSUBSCRIBE_URL, token_url);
    let per_hour =
        reader.get_with(&PER_RECIPIENT_PER_HOUR, |raw| parse_in_range(raw, 1u32, 10_000));
    Ok(NotificationsConfig {
        retention_days: retention?,
        inbox_retention_days: inbox?,
        unsubscribe_url: url?,
        per_recipient_per_hour: per_hour?,
    })
}

fn token_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    if (lower.starts_with("https://") || lower.starts_with("http://")) && url.contains("{token}") {
        Ok(url.to_owned())
    } else {
        Err("must be an http(s) URL with `{token}` where the token goes".to_owned())
    }
}
