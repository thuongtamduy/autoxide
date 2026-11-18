//! Cấu hình của module `shipping`, tiền tố `AUTOXIDE_SHIPPING_`. Mọi biến có mặc định chạy được
//! ngay. Cấu hình của từng hãng thuộc từng khách nằm trong `shipping_carrier_accounts` (thông tin
//! đăng nhập mã hoá), lịch giao / ngày nghỉ / giờ chốt nằm trong bảng — không trong biến môi
//! trường (D-190, D-201).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const FAKE_CARRIER: Setting = Setting::new(
    "AUTOXIDE_SHIPPING_FAKE_CARRIER",
    "Register the in-memory fake carrier (`fake`) for trying shipping on a development machine. \
     Never turn on in production: `autoxide doctor` warns when it is on.",
)
.with_default("false");

pub(crate) const CARRIER_TIMEOUT_SECONDS: Setting = Setting::new(
    "AUTOXIDE_SHIPPING_CARRIER_TIMEOUT_SECONDS",
    "Seconds to wait for a shipping carrier before giving up on one call (1 to 120). A quote \
     lists a carrier that did not answer in time as unavailable instead of failing.",
)
.with_default("10");

pub(crate) const MAX_ADDRESSES_PER_BUYER: Setting = Setting::new(
    "AUTOXIDE_SHIPPING_MAX_ADDRESSES_PER_BUYER",
    "Addresses one buyer may keep in their address book (1 to 200).",
)
.with_default("20");

pub(crate) const WEBHOOK_MAX_BYTES: Setting = Setting::new(
    "AUTOXIDE_SHIPPING_WEBHOOK_MAX_BYTES",
    "Largest carrier webhook body accepted, in bytes (1024 to 10485760); larger bodies get 413.",
)
.with_default("262144");

pub(crate) const COD_SETTLEMENT_DAYS: Setting = Setting::new(
    "AUTOXIDE_SHIPPING_COD_SETTLEMENT_DAYS",
    "Days after delivery by which a cash-on-delivery amount should appear on a carrier \
     statement; later, it is reported as overdue (1 to 180).",
)
.with_default("14");

/// Cấu hình đã đọc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShippingConfig {
    pub fake_carrier: bool,
    pub carrier_timeout_seconds: u64,
    pub max_addresses_per_buyer: u64,
    pub webhook_max_bytes: usize,
    pub cod_settlement_days: i64,
}

impl Default for ShippingConfig {
    fn default() -> Self {
        ShippingConfig {
            fake_carrier: false,
            carrier_timeout_seconds: 10,
            max_addresses_per_buyer: 20,
            webhook_max_bytes: 262_144,
            cod_settlement_days: 14,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<ShippingConfig, Invalid> {
    let fake = reader.get::<bool>(&FAKE_CARRIER);
    let timeout = reader.get_with(&CARRIER_TIMEOUT_SECONDS, |raw| parse_in_range(raw, 1u64, 120));
    let addresses = reader.get_with(&MAX_ADDRESSES_PER_BUYER, |raw| parse_in_range(raw, 1u64, 200));
    let webhook =
        reader.get_with(&WEBHOOK_MAX_BYTES, |raw| parse_in_range(raw, 1024usize, 10_485_760));
    let cod = reader.get_with(&COD_SETTLEMENT_DAYS, |raw| parse_in_range(raw, 1i64, 180));
    Ok(ShippingConfig {
        fake_carrier: fake?,
        carrier_timeout_seconds: timeout?,
        max_addresses_per_buyer: addresses?,
        webhook_max_bytes: webhook?,
        cod_settlement_days: cod?,
    })
}
