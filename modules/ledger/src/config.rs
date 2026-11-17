//! Cấu hình của module `ledger`. Cấu hình theo khách (cổng thanh toán, chế độ dòng tiền) là dữ liệu
//! trong bảng, không phải biến môi trường.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const PAYMENT_EXPIRY_MINUTES: Setting = Setting::new(
    "AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES",
    "Minutes a payment waits for the buyer before it expires (also VietQR transfers), 1 to 10080.",
)
.with_default("30");

pub(crate) const WEBHOOK_MAX_BYTES: Setting = Setting::new(
    "AUTOXIDE_LEDGER_WEBHOOK_MAX_BYTES",
    "Largest body accepted from a payment partner webhook, in bytes, 1024 to 1048576.",
)
.with_default("65536");

pub(crate) const BANK_FEED_POLL_SECONDS: Setting = Setting::new(
    "AUTOXIDE_LEDGER_BANK_FEED_POLL_SECONDS",
    "Seconds between two reads of the bank feeds (VietQR transfers), 10 to 86400.",
)
.with_default("60");

/// Cấu hình đã đọc.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LedgerConfig {
    pub payment_expiry_minutes: i64,
    pub webhook_max_bytes: usize,
    pub bank_feed_poll_seconds: u64,
}

impl Default for LedgerConfig {
    fn default() -> Self {
        LedgerConfig {
            payment_expiry_minutes: 30,
            webhook_max_bytes: 65_536,
            bank_feed_poll_seconds: 60,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<LedgerConfig, Invalid> {
    Ok(LedgerConfig {
        payment_expiry_minutes: reader
            .get_with(&PAYMENT_EXPIRY_MINUTES, |raw| parse_in_range(raw, 1i64, 10_080))?,
        webhook_max_bytes: reader
            .get_with(&WEBHOOK_MAX_BYTES, |raw| parse_in_range(raw, 1024usize, 1_048_576))?,
        bank_feed_poll_seconds: reader
            .get_with(&BANK_FEED_POLL_SECONDS, |raw| parse_in_range(raw, 10u64, 86_400))?,
    })
}
