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

/// Cấu hình đã đọc.
#[derive(Debug, Clone)]
pub(crate) struct CustomersConfig {
    pub wishlist_max: u64,
    pub recent_views_max: u64,
    pub recent_views_days: i64,
    pub referral_apply_days: i64,
}

impl Default for CustomersConfig {
    fn default() -> Self {
        CustomersConfig {
            wishlist_max: 200,
            recent_views_max: 50,
            recent_views_days: 90,
            referral_apply_days: 14,
        }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<CustomersConfig, Invalid> {
    let wishlist = reader.get_with(&WISHLIST_MAX, |raw| parse_in_range(raw, 1u64, 10_000));
    let views = reader.get_with(&RECENT_VIEWS_MAX, |raw| parse_in_range(raw, 1u64, 1000));
    let days = reader.get_with(&RECENT_VIEWS_DAYS, |raw| parse_in_range(raw, 1i64, 3650));
    let referral = reader.get_with(&REFERRAL_APPLY_DAYS, |raw| parse_in_range(raw, 1i64, 365));
    Ok(CustomersConfig {
        wishlist_max: wishlist?,
        recent_views_max: views?,
        recent_views_days: days?,
        referral_apply_days: referral?,
    })
}
