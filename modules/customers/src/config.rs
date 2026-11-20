//! Cấu hình của module `customers`, tiền tố `AUTOXIDE_CUSTOMERS_`. Mọi biến có mặc định chạy được
//! ngay. Quy tắc kinh doanh (điểm, hạng, nhắc giỏ, giới thiệu) là dữ liệu có ngày hiệu lực, không
//! phải biến môi trường.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const WISHLIST_MAX: Setting = Setting::new(
    "AUTOXIDE_CUSTOMERS_WISHLIST_MAX",
    "Most products a buyer keeps in the wishlist (1 to 10000).",
)
.with_default("200");

pub(crate) const RECENT_VIEWS_MAX: Setting = Setting::new(
    "AUTOXIDE_CUSTOMERS_RECENT_VIEWS_MAX",
    "Most recently viewed products kept per signed-in buyer (1 to 1000).",
)
.with_default("50");

pub(crate) const RECENT_VIEWS_DAYS: Setting = Setting::new(
    "AUTOXIDE_CUSTOMERS_RECENT_VIEWS_DAYS",
    "Days a recently viewed product is kept (1 to 3650).",
)
.with_default("90");

pub(crate) const REFERRAL_APPLY_DAYS: Setting = Setting::new(
    "AUTOXIDE_CUSTOMERS_REFERRAL_APPLY_DAYS",
    "Days after signing up during which a buyer may enter a referral code, before the first order \
     (1 to 365).",
)
.with_default("14");

pub(crate) const STORED_VALUE_ENABLED: Setting = Setting::new(
    "AUTOXIDE_CUSTOMERS_STORED_VALUE_ENABLED",
    "Turn on gift cards and store credit (`true` / `false`). Off by default: turn on only after a \
     qualified person checked Decree 52/2024/ND-CP on prepaid cards; each shop also needs the plan \
     features `customers.gift_cards` / `customers.store_credit`.",
)
.with_default("false");

pub(crate) const GIFT_CARD_MAX_MINOR: Setting = Setting::new(
    "AUTOXIDE_CUSTOMERS_GIFT_CARD_MAX_MINOR",
    "Largest value of one gift card, in the smallest currency unit (1 to 1000000000000).",
)
.with_default("10000000");

pub(crate) const GIFT_CARD_LOOKUPS_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_CUSTOMERS_GIFT_CARD_LOOKUPS_PER_HOUR",
    "Wrong gift card codes one person may enter per hour before lookups are blocked for the hour, \
     against guessing codes (1 to 1000).",
)
.with_default("10");

pub(crate) const WISHLIST_PRICE_DROP_MIN_BP: Setting = Setting::new(
    "AUTOXIDE_CUSTOMERS_WISHLIST_PRICE_DROP_MIN_BP",
    "Smallest price drop that triggers a wishlist alert, in basis points of the price when the \
     product was added (100 = 1%; 1 to 9000).",
)
.with_default("500");

pub(crate) const WISHLIST_ALERTS_PER_DAY: Setting = Setting::new(
    "AUTOXIDE_CUSTOMERS_WISHLIST_ALERTS_PER_DAY",
    "Most wishlist alert messages one buyer gets per day, each with up to 10 products (1 to 10).",
)
.with_default("1");

/// Cấu hình đã đọc.
#[derive(Debug, Clone)]
pub(crate) struct CustomersConfig {
    pub wishlist_max: u64,
    pub recent_views_max: u64,
    pub recent_views_days: i64,
    pub referral_apply_days: i64,
    pub stored_value_enabled: bool,
    pub gift_card_max_minor: i64,
    pub lookup_per_hour: u32,
    pub price_drop_min_bp: i64,
    pub wishlist_alerts_per_day: u64,
}

impl Default for CustomersConfig {
    fn default() -> Self {
        CustomersConfig {
            wishlist_max: 200,
            recent_views_max: 50,
            recent_views_days: 90,
            referral_apply_days: 14,
            stored_value_enabled: false,
            gift_card_max_minor: 10_000_000,
            lookup_per_hour: 10,
            price_drop_min_bp: 500,
            wishlist_alerts_per_day: 1,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<CustomersConfig, Invalid> {
    let wishlist = reader.get_with(&WISHLIST_MAX, |raw| parse_in_range(raw, 1u64, 10_000));
    let views = reader.get_with(&RECENT_VIEWS_MAX, |raw| parse_in_range(raw, 1u64, 1000));
    let days = reader.get_with(&RECENT_VIEWS_DAYS, |raw| parse_in_range(raw, 1i64, 3650));
    let referral = reader.get_with(&REFERRAL_APPLY_DAYS, |raw| parse_in_range(raw, 1i64, 365));
    let enabled = reader.get_with(&STORED_VALUE_ENABLED, |raw| match raw.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("must be `true` or `false`".to_owned()),
    });
    let max =
        reader.get_with(&GIFT_CARD_MAX_MINOR, |raw| parse_in_range(raw, 1i64, 1_000_000_000_000));
    let lookups =
        reader.get_with(&GIFT_CARD_LOOKUPS_PER_HOUR, |raw| parse_in_range(raw, 1u32, 1000));
    let drop = reader.get_with(&WISHLIST_PRICE_DROP_MIN_BP, |raw| parse_in_range(raw, 1i64, 9000));
    let per_day = reader.get_with(&WISHLIST_ALERTS_PER_DAY, |raw| parse_in_range(raw, 1u64, 10));
    Ok(CustomersConfig {
        wishlist_max: wishlist?,
        recent_views_max: views?,
        recent_views_days: days?,
        referral_apply_days: referral?,
        stored_value_enabled: enabled?,
        gift_card_max_minor: max?,
        lookup_per_hour: lookups?,
        price_drop_min_bp: drop?,
        wishlist_alerts_per_day: per_day?,
    })
}
