//! Cấu hình của module `orders`, tiền tố `AUTOXIDE_ORDERS_`. Mọi biến có mặc định chạy được ngay.
//! Chính sách kinh doanh (hạn đổi trả, huỷ, COD, hoá đơn, đặt trước…) là dữ liệu theo nút trong
//! bảng, không phải biến môi trường (đặc tả 45 mục "Cấu hình"). Hạn thanh toán không có biến riêng:
//! đọc từ `ledger` (D-218).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const CART_TTL_DAYS: Setting = Setting::new(
    "AUTOXIDE_ORDERS_CART_TTL_DAYS",
    "Days a cart is kept after its last change before it is deleted (1 to 365).",
)
.with_default("30");

pub(crate) const CART_MAX_LINES: Setting = Setting::new(
    "AUTOXIDE_ORDERS_CART_MAX_LINES",
    "Different products one cart may hold (1 to 500).",
)
.with_default("100");

pub(crate) const LINE_MAX_QUANTITY: Setting = Setting::new(
    "AUTOXIDE_ORDERS_LINE_MAX_QUANTITY",
    "Largest quantity of one product in a cart (1 to 100000).",
)
.with_default("999");

pub(crate) const ABANDONED_AFTER_MINUTES: Setting = Setting::new(
    "AUTOXIDE_ORDERS_ABANDONED_AFTER_MINUTES",
    "Minutes without change after which a cart of a known buyer counts as abandoned (5 to 10080).",
)
.with_default("60");

pub(crate) const QUOTE_TTL_MINUTES: Setting = Setting::new(
    "AUTOXIDE_ORDERS_QUOTE_TTL_MINUTES",
    "Minutes a cart quote (`quote_token`) stays valid for checkout (1 to 1440).",
)
.with_default("15");

/// Cấu hình đã đọc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrdersConfig {
    pub cart_ttl_days: i64,
    pub cart_max_lines: usize,
    pub line_max_quantity: i64,
    pub abandoned_after_minutes: i64,
    pub quote_ttl_minutes: i64,
}

impl Default for OrdersConfig {
    fn default() -> Self {
        OrdersConfig {
            cart_ttl_days: 30,
            cart_max_lines: 100,
            line_max_quantity: 999,
            abandoned_after_minutes: 60,
            quote_ttl_minutes: 15,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<OrdersConfig, Invalid> {
    let ttl = reader.get_with(&CART_TTL_DAYS, |raw| parse_in_range(raw, 1i64, 365));
    let lines = reader.get_with(&CART_MAX_LINES, |raw| parse_in_range(raw, 1usize, 500));
    let quantity = reader.get_with(&LINE_MAX_QUANTITY, |raw| parse_in_range(raw, 1i64, 100_000));
    let abandoned =
        reader.get_with(&ABANDONED_AFTER_MINUTES, |raw| parse_in_range(raw, 5i64, 10_080));
    let quote = reader.get_with(&QUOTE_TTL_MINUTES, |raw| parse_in_range(raw, 1i64, 1440));
    Ok(OrdersConfig {
        cart_ttl_days: ttl?,
        cart_max_lines: lines?,
        line_max_quantity: quantity?,
        abandoned_after_minutes: abandoned?,
        quote_ttl_minutes: quote?,
    })
}
