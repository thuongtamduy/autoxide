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

pub(crate) const HOLD_MINUTES: Setting = Setting::new(
    "AUTOXIDE_ORDERS_HOLD_MINUTES",
    "Minutes stock and promotion uses stay held for an unpaid order (15 to 1440). Keep it longer \
     than the payment deadline of the ledger (`AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES`) plus \
     `AUTOXIDE_ORDERS_HOLD_GRACE_MINUTES`, so that unpaid orders are cancelled before holds lapse.",
)
.with_default("60");

pub(crate) const HOLD_GRACE_MINUTES: Setting = Setting::new(
    "AUTOXIDE_ORDERS_HOLD_GRACE_MINUTES",
    "Minutes holds outlive the payment deadline (1 to 120).",
)
.with_default("10");

pub(crate) const CHECKOUT_STUCK_MINUTES: Setting = Setting::new(
    "AUTOXIDE_ORDERS_CHECKOUT_STUCK_MINUTES",
    "Minutes after which a checkout stuck while holding stock is undone and its holds released \
     (1 to 1440).",
)
.with_default("10");

pub(crate) const LOOKUP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_ORDERS_LOOKUP_PER_HOUR",
    "Guest order lookups (each sends a one-time code by SMS) allowed per hour for one order number \
     and phone number (1 to 100).",
)
.with_default("5");

pub(crate) const LOOKUP_IP_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_ORDERS_LOOKUP_IP_PER_HOUR",
    "Guest order lookups allowed per hour from one client address (1 to 1000).",
)
.with_default("20");

pub(crate) const LOOKUP_RESEND_SECONDS: Setting = Setting::new(
    "AUTOXIDE_ORDERS_LOOKUP_RESEND_SECONDS",
    "Seconds a guest waits before asking for another one-time code for the same order (10 to \
     3600).",
)
.with_default("60");

pub(crate) const LOOKUP_TOKEN_MINUTES: Setting = Setting::new(
    "AUTOXIDE_ORDERS_LOOKUP_TOKEN_MINUTES",
    "Minutes a guest order access token (`Order-Token`) stays valid after the one-time code (5 to \
     1440).",
)
.with_default("30");

pub(crate) const REVIEW_EDIT_DAYS: Setting = Setting::new(
    "AUTOXIDE_ORDERS_REVIEW_EDIT_DAYS",
    "Days a buyer may still edit a product review after writing it (0 to 365).",
)
.with_default("30");

pub(crate) const REVIEW_VOTES_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_ORDERS_REVIEW_VOTES_PER_HOUR",
    "\"Helpful\" votes one account may cast per hour (1 to 10000).",
)
.with_default("60");

/// Cấu hình đã đọc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrdersConfig {
    pub cart_ttl_days: i64,
    pub cart_max_lines: usize,
    pub line_max_quantity: i64,
    pub abandoned_after_minutes: i64,
    pub quote_ttl_minutes: i64,
    pub hold_minutes: i64,
    pub hold_grace_minutes: i64,
    pub checkout_stuck_minutes: i64,
    pub lookup_per_hour: u32,
    pub lookup_ip_per_hour: u32,
    pub lookup_resend_seconds: i64,
    pub lookup_token_minutes: i64,
    pub review_edit_days: i64,
    pub review_votes_per_hour: u32,
}

impl Default for OrdersConfig {
    fn default() -> Self {
        OrdersConfig {
            cart_ttl_days: 30,
            cart_max_lines: 100,
            line_max_quantity: 999,
            abandoned_after_minutes: 60,
            quote_ttl_minutes: 15,
            hold_minutes: 60,
            hold_grace_minutes: 10,
            checkout_stuck_minutes: 10,
            lookup_per_hour: 5,
            lookup_ip_per_hour: 20,
            lookup_resend_seconds: 60,
            lookup_token_minutes: 30,
            review_edit_days: 30,
            review_votes_per_hour: 60,
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
    let hold = reader.get_with(&HOLD_MINUTES, |raw| parse_in_range(raw, 15i64, 1440));
    let grace = reader.get_with(&HOLD_GRACE_MINUTES, |raw| parse_in_range(raw, 1i64, 120));
    let stuck = reader.get_with(&CHECKOUT_STUCK_MINUTES, |raw| parse_in_range(raw, 1i64, 1440));
    let lookups = reader.get_with(&LOOKUP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 100));
    let lookups_ip = reader.get_with(&LOOKUP_IP_PER_HOUR, |raw| parse_in_range(raw, 1u32, 1000));
    let resend = reader.get_with(&LOOKUP_RESEND_SECONDS, |raw| parse_in_range(raw, 10i64, 3600));
    let token = reader.get_with(&LOOKUP_TOKEN_MINUTES, |raw| parse_in_range(raw, 5i64, 1440));
    let review_edit = reader.get_with(&REVIEW_EDIT_DAYS, |raw| parse_in_range(raw, 0i64, 365));
    let votes = reader.get_with(&REVIEW_VOTES_PER_HOUR, |raw| parse_in_range(raw, 1u32, 10_000));
    Ok(OrdersConfig {
        cart_ttl_days: ttl?,
        cart_max_lines: lines?,
        line_max_quantity: quantity?,
        abandoned_after_minutes: abandoned?,
        quote_ttl_minutes: quote?,
        hold_minutes: hold?,
        hold_grace_minutes: grace?,
        checkout_stuck_minutes: stuck?,
        lookup_per_hour: lookups?,
        lookup_ip_per_hour: lookups_ip?,
        lookup_resend_seconds: resend?,
        lookup_token_minutes: token?,
        review_edit_days: review_edit?,
        review_votes_per_hour: votes?,
    })
}
