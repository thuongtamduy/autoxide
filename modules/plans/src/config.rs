//! Cấu hình của module `plans`.

use jiff::SignedDuration;
use std::time::Duration;

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const CACHE_SECONDS: Setting = Setting::new(
    "AUTOXIDE_PLANS_CACHE_SECONDS",
    "How long each server keeps what a tenant may use (its plan, add-ons, overrides) in memory, \
     0 to 300 seconds. A change made on another server takes effect here within this time; `0` \
     reads it on every request.",
)
.with_default("30");

pub(crate) const TAX_CATEGORY: Setting = Setting::new(
    "AUTOXIDE_PLANS_TAX_CATEGORY",
    "Tax category of the subscription line on the invoices the operator issues to tenants \
     (`GET /api/v1/tax/categories`). Plan prices include this tax.",
)
.with_default("vat.standard");

pub(crate) const HOLD_MINUTES: Setting = Setting::new(
    "AUTOXIDE_PLANS_HOLD_MINUTES",
    "How long a reservation against a limit is held before it is released by itself when the \
     module that made it never confirms it (for example the server stopped), 1 to 1440 minutes.",
)
.with_default("15");

pub(crate) const REMINDER_DAYS: Setting = Setting::new(
    "AUTOXIDE_PLANS_REMINDER_DAYS",
    "Days before a billing period is due that a reminder is mailed to the subscription's billing \
     email, separated by commas (each 1 to 60). A reminder is also sent when the period is overdue \
     and every 3 days of the grace period.",
)
.with_default("7,1");

pub(crate) const SIGNUP: Setting = Setting::new(
    "AUTOXIDE_PLANS_SIGNUP",
    "`true` lets new tenants sign up by themselves (`POST /api/v1/plans/signups`) on plans marked \
     `self_signup`; `false` (the default) answers `plans.signup_disabled` and only platform staff \
     create tenants.",
)
.with_default("false");

pub(crate) const SIGNUP_URL: Setting = Setting::new(
    "AUTOXIDE_PLANS_SIGNUP_URL",
    "Page of the sign-up website that confirms a sign-up, with `{token}` where the token goes; the \
     page sends it to `POST /api/v1/plans/signups/confirm`. Without it the confirmation mail \
     carries the token itself.",
)
.with_example("https://www.example.com/signup/confirm?token={token}");

pub(crate) const SIGNUP_RETURN_URL: Setting = Setting::new(
    "AUTOXIDE_PLANS_SIGNUP_RETURN_URL",
    "Page the identity provider sends people back to after the identity checks of a sign-up \
     (plans that `requires_kyc`); `{id}` is replaced by the sign-up id. Optional.",
)
.with_example("https://www.example.com/signup/{id}/checked");

/// Cấu hình đăng ký công khai (D-349).
#[derive(Debug, Clone, Default)]
pub(crate) struct SignupConfig {
    pub enabled: bool,
    /// Trang xác nhận có `{token}`; `None`: mail chứa token.
    pub url: Option<String>,
    /// Trang quay về sau định danh, có thể có `{id}`.
    pub return_url: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PlansConfig {
    pub tax_category: String,
    pub cache: Duration,
    pub hold: SignedDuration,
    pub reminder_days: Vec<u32>,
    pub signup: SignupConfig,
}

impl PlansConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let cache = reader.get_with(&CACHE_SECONDS, |raw| parse_in_range(raw, 0u64, 300));
        let hold = reader.get_with(&HOLD_MINUTES, |raw| parse_in_range(raw, 1i64, 1440));
        let reminder_days = reader.get_with(&REMINDER_DAYS, |raw| {
            let mut days = Vec::new();
            for part in raw.split(',').map(str::trim).filter(|part| !part.is_empty()) {
                days.push(parse_in_range(part, 1u32, 60)?);
            }
            Ok(days)
        });
        let tax_category = reader.get_with(&TAX_CATEGORY, |raw| {
            let valid = (1..=64).contains(&raw.len())
                && raw.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_'
                });
            if valid {
                Ok(raw.to_owned())
            } else {
                Err("must be a tax category code, e.g. `vat.standard`".to_owned())
            }
        });
        let enabled = reader.get::<bool>(&SIGNUP);
        let url = reader.optional_with(&SIGNUP_URL, |raw| http_url(raw, Some("{token}")));
        let return_url = reader.optional_with(&SIGNUP_RETURN_URL, |raw| http_url(raw, None));
        Ok(PlansConfig {
            tax_category: tax_category?,
            cache: Duration::from_secs(cache?),
            hold: SignedDuration::from_mins(hold?),
            reminder_days: reminder_days?,
            signup: SignupConfig { enabled: enabled?, url: url?, return_url: return_url? },
        })
    }
}

/// Địa chỉ http(s); `needs`: chuỗi phải có (vd `{token}`).
fn http_url(raw: &str, needs: Option<&str>) -> Result<String, String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    let scheme = lower.starts_with("https://") || lower.starts_with("http://");
    match needs {
        Some(part) if !(scheme && url.contains(part)) => {
            Err(format!("must be an http(s) URL containing `{part}`"))
        }
        None if !scheme => Err("must be an http(s) URL".to_owned()),
        _ => Ok(url.to_owned()),
    }
}
