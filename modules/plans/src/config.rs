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

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlansConfig {
    pub cache: Duration,
    pub hold: chrono::Duration,
}

impl PlansConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let cache = reader.get_with(&CACHE_SECONDS, |raw| parse_in_range(raw, 0u64, 300));
        let hold = reader.get_with(&HOLD_MINUTES, |raw| parse_in_range(raw, 1i64, 1440));
        Ok(PlansConfig {
            cache: Duration::from_secs(cache?),
            hold: chrono::Duration::minutes(hold?),
        })
    }
}
