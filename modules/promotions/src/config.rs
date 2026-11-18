//! Cấu hình của module `promotions`, tiền tố `AUTOXIDE_PROMOTIONS_`. Mọi biến có mặc định chạy
//! được ngay. Luật cộng dồn và thu lại khi trả hàng là cài đặt kế thừa theo nút
//! (`promotions.stacking_mode`, `promotions.clawback_on_return`), không phải biến môi trường.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const MAX_CODES_PER_CART: Setting = Setting::new(
    "AUTOXIDE_PROMOTIONS_MAX_CODES_PER_CART",
    "Coupon codes a buyer may enter on one cart (1 to 20).",
)
.with_default("5");

pub(crate) const MAX_HOLD_HOURS: Setting = Setting::new(
    "AUTOXIDE_PROMOTIONS_MAX_HOLD_HOURS",
    "Longest time an order may hold promotion usage before it is confirmed or released, in \
     hours (1 to 720).",
)
.with_default("72");

pub(crate) const CODE_LENGTH: Setting = Setting::new(
    "AUTOXIDE_PROMOTIONS_CODE_LENGTH",
    "Length of generated codes (8 to 32 characters, letters and digits that cannot be mistaken \
     for each other).",
)
.with_default("10");

pub(crate) const MAX_VOUCHERS_PER_CLASS: Setting = Setting::new(
    "AUTOXIDE_PROMOTIONS_MAX_VOUCHERS_PER_CLASS",
    "Vouchers one cart may use per promotion class (product, order, shipping) and per payer \
     (1 to 5).",
)
.with_default("1");

pub(crate) const COMPILED_TTL_SECONDS: Setting = Setting::new(
    "AUTOXIDE_PROMOTIONS_COMPILED_TTL_SECONDS",
    "Seconds a server keeps the running promotions in memory before it checks for changes made \
     on another server (1 to 3600).",
)
.with_default("60");

pub(crate) const JURISDICTION: Setting = Setting::new(
    "AUTOXIDE_PROMOTIONS_JURISDICTION",
    "Country whose promotion law applies (two-letter code of the legal data pack, e.g. `VN`).",
)
.with_default("VN");

/// Cấu hình đã đọc.
#[derive(Debug, Clone)]
pub(crate) struct PromotionsConfig {
    pub max_codes_per_cart: usize,
    pub max_hold_hours: i64,
    pub code_length: usize,
    pub max_vouchers_per_class: usize,
    pub compiled_ttl_seconds: u64,
    pub jurisdiction: String,
}

impl Default for PromotionsConfig {
    fn default() -> Self {
        PromotionsConfig {
            max_codes_per_cart: 5,
            max_hold_hours: 72,
            code_length: 10,
            max_vouchers_per_class: 1,
            compiled_ttl_seconds: 60,
            jurisdiction: "VN".to_owned(),
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<PromotionsConfig, Invalid> {
    let codes = reader.get_with(&MAX_CODES_PER_CART, |raw| parse_in_range(raw, 1usize, 20));
    let hold = reader.get_with(&MAX_HOLD_HOURS, |raw| parse_in_range(raw, 1i64, 720));
    let length = reader.get_with(&CODE_LENGTH, |raw| parse_in_range(raw, 8usize, 32));
    let vouchers = reader.get_with(&MAX_VOUCHERS_PER_CLASS, |raw| parse_in_range(raw, 1usize, 5));
    let ttl = reader.get_with(&COMPILED_TTL_SECONDS, |raw| parse_in_range(raw, 1u64, 3600));
    let jurisdiction = reader.get_with(&JURISDICTION, |raw| {
        let code = raw.trim().to_ascii_uppercase();
        if code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()) {
            Ok(code)
        } else {
            Err("must be a two-letter country code like `VN`".to_owned())
        }
    });
    Ok(PromotionsConfig {
        max_codes_per_cart: codes?,
        max_hold_hours: hold?,
        code_length: length?,
        max_vouchers_per_class: vouchers?,
        compiled_ttl_seconds: ttl?,
        jurisdiction: jurisdiction?,
    })
}
