//! Cấu hình của module `plans`.

use std::time::Duration;

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const CACHE_SECONDS: Setting = Setting::new(
    "AUTOXIDE_PLANS_CACHE_SECONDS",
    "How long each server keeps what a tenant may use (its plan, add-ons, overrides) in memory, \
     0 to 300 seconds. A change made on another server takes effect here within this time; `0` \
     reads it on every request.",
)
.with_default("30");

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

#[derive(Debug, Clone)]
pub(crate) struct PlansConfig {
    pub cache: Duration,
    pub hold: chrono::Duration,
    pub reminder_days: Vec<u32>,
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
        Ok(PlansConfig {
            cache: Duration::from_secs(cache?),
            hold: chrono::Duration::minutes(hold?),
            reminder_days: reminder_days?,
        })
    }
}
