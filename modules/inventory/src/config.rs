//! Cấu hình của module `inventory`, tiền tố `AUTOXIDE_INVENTORY_`. Mọi biến có mặc định chạy được
//! ngay. Quy tắc theo khách (kho phục vụ cửa hàng, chia kho, tồn tối thiểu) nằm trong bảng của
//! module, không ở biến môi trường.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const MAX_HOLD_HOURS: Setting = Setting::new(
    "AUTOXIDE_INVENTORY_MAX_HOLD_HOURS",
    "Longest time stock may stay reserved for an unpaid order, in hours (1 to 720).",
)
.with_default("72");

pub(crate) const EXPIRE_BATCH: Setting = Setting::new(
    "AUTOXIDE_INVENTORY_EXPIRE_BATCH",
    "Expired reservations released per run of the minute job (10 to 10000).",
)
.with_default("200");

pub(crate) const MAX_LINES_PER_RESERVATION: Setting = Setting::new(
    "AUTOXIDE_INVENTORY_MAX_LINES_PER_RESERVATION",
    "Most lines one reservation (one order) may hold (1 to 1000).",
)
.with_default("200");

pub(crate) const PUBLIC_LOW_THRESHOLD: Setting = Setting::new(
    "AUTOXIDE_INVENTORY_PUBLIC_LOW_THRESHOLD",
    "Below this available quantity the public availability says `low` and shows the quantity; \
     at or above it only `in_stock` is shown, never the number (0 to 1000).",
)
.with_default("5");

/// Cấu hình đã đọc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InventoryConfig {
    pub max_hold_hours: i64,
    pub expire_batch: u64,
    pub max_lines_per_reservation: usize,
    pub public_low_threshold: i64,
}

impl Default for InventoryConfig {
    fn default() -> Self {
        InventoryConfig {
            max_hold_hours: 72,
            expire_batch: 200,
            max_lines_per_reservation: 200,
            public_low_threshold: 5,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<InventoryConfig, Invalid> {
    let hold = reader.get_with(&MAX_HOLD_HOURS, |raw| parse_in_range(raw, 1i64, 720));
    let batch = reader.get_with(&EXPIRE_BATCH, |raw| parse_in_range(raw, 10u64, 10_000));
    let lines =
        reader.get_with(&MAX_LINES_PER_RESERVATION, |raw| parse_in_range(raw, 1usize, 1000));
    let low = reader.get_with(&PUBLIC_LOW_THRESHOLD, |raw| parse_in_range(raw, 0i64, 1000));
    Ok(InventoryConfig {
        max_hold_hours: hold?,
        expire_batch: batch?,
        max_lines_per_reservation: lines?,
        public_low_threshold: low?,
    })
}
