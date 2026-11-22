//! Settings of the module. Each is declared once as a `Setting` and read through `ConfigReader`
//! (never `std::env::var`), so it shows up in `acme config`, in the generated `.env.example`, and
//! a wrong value stops every command with one clear message.
//!
//! Names: the official modules use the prefix `AUTOXIDE_<MODULE>_`. A module of your own uses a
//! prefix of your own, so that it can never clash with a setting the core adds later.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const RETENTION_DAYS: Setting = Setting::new(
    "ACME_BOOKMARKS_RETENTION_DAYS",
    "Days a bookmark is kept before a daily job deletes it (0 to 36500; 0 keeps bookmarks until \
     their owner deletes them).",
)
.with_default("0");

#[derive(Debug, Clone, Default)]
pub(crate) struct BookmarksConfig {
    /// 0: keep until deleted by hand.
    pub retention_days: u32,
}

impl BookmarksConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        // Read every setting before the first `?`, so that all wrong values are reported at once.
        let retention_days =
            reader.get_with(&RETENTION_DAYS, |raw| parse_in_range(raw, 0u32, 36_500));
        Ok(BookmarksConfig { retention_days: retention_days? })
    }
}
