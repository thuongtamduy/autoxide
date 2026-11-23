//! Time in Autoxide (D-20261121-time-library): [`Timestamp`] (a point in time, UTC) and [`Date`] (a calendar
//! day, no time of day), built on `jiff`.
//!
//! Why the kernel wraps the library's types instead of letting modules use them directly:
//!
//! - **The on-disk form does not change.** Time columns used to be written with `chrono` through `sqlx`:
//!   SQLite stores the string `2026-11-21T10:00:00.123456+00:00` (fraction of 0, 3, 6 or 9 digits, suffix
//!   `+00:00`), Postgres stores `timestamptz`. The `jiff` / `time` support built into `sqlx` / `sea-query`
//!   writes SQLite in another form (`…Z`, trailing zeros cut), and SQLite compares times as strings — mixing
//!   the two forms gives wrong ordering and range filtering. The types here encode the old form themselves
//!   (see `sql.rs`), so old and new data in the same column still compare correctly.
//! - **JSON does not change**: RFC 3339, UTC, suffix `Z`, fraction of 0/3/6/9 digits — as `chrono` returned before.
//! - **OpenAPI does not change**: `utoipa` recognizes the types by NAME (`Timestamp` → `string`, `date-time`;
//!   `Date` → `string`, `date`), so nothing more needs declaring on each field.
//! - **Both databases behave the same**: [`Timestamp::now`] truncates to microseconds — Postgres's precision
//!   — so a value just created in memory equals the value read back from either database.
//!
//! **Hand-written SQL:** values are written as strings together with the type name `timestamptz` / `date`;
//! Postgres accepts them only when the statement carries that type name (`$1::"timestamptz"`). The SeaORM /
//! sea-query builders insert it themselves; hand-written statements are built with
//! `autoxide_kernel::database::statement` (the `raw-statement` rule of `cargo xtask check` rejects
//! `Statement::from_sql_and_values`). Do not write `CAST(? AS timestamptz)` yourself: SQLite reads an unknown
//! type as a number.
//!
//! Time zones, durations and calendar computation (adding months, weekday…) use `jiff` directly
//! (`jiff::tz::TimeZone`, `jiff::SignedDuration`, `jiff::civil`): there is nothing to store or return, so no
//! wrapper is needed.
//!
//! `chrono` / `chrono-tz` have been removed entirely; tests compare against `chrono` values recorded in `time/golden.json`.

mod date;
mod format;
mod sql;
mod timestamp;

#[cfg(test)]
mod tests;

use std::fmt;

pub use date::Date;
/// A signed duration (`jiff::SignedDuration`), re-exported so modules can add to and subtract from
/// [`Timestamp`] (`Timestamp::now() - SignedDuration::from_hours(24)`) without declaring `jiff` themselves
/// (D-20261122-module-helpers).
pub use jiff::SignedDuration;
pub use timestamp::Timestamp;

/// An IANA time zone by name (`Asia/Ho_Chi_Minh`, `UTC`), taken from the time zone data packaged in the
/// binary (independent of the server). The name must use the same casing as in the data — as `chrono-tz` did
/// before; `jiff` itself also accepts `asia/ho_chi_minh`, but a stored name is read back and matched
/// elsewhere, so keep one form.
pub fn zone(name: &str) -> Option<jiff::tz::TimeZone> {
    let zone = jiff::tz::TimeZone::get(name).ok()?;
    (zone.iana_name() == Some(name)).then_some(zone)
}

/// The point in time of local time `local` in zone `zone` — like `chrono`
/// `from_local_datetime(…).earliest()`: a local time that does not exist (daylight saving jump) gives `None`,
/// a repeated time (clocks set back) takes the earlier one.
pub fn from_local(zone: &jiff::tz::TimeZone, local: jiff::civil::DateTime) -> Option<Timestamp> {
    use jiff::tz::AmbiguousOffset;
    let offset = match zone.to_ambiguous_timestamp(local).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset,
        // Hai lần cùng giờ: lần đầu còn mang độ lệch trước khi đổi giờ.
        AmbiguousOffset::Fold { before, .. } => before,
        AmbiguousOffset::Gap { .. } => return None,
    };
    offset.to_timestamp(local).ok().map(Timestamp::from)
}

/// A time of day written `HH:MM` (`09:00`, `9:00` is also accepted), like opening hours. Seconds and `24:00` are not accepted.
pub fn parse_clock(raw: &str) -> Option<jiff::civil::Time> {
    jiff::civil::Time::strptime("%H:%M", raw).ok()
}

/// Number of fractional digits of the seconds when writing RFC 3339 (like `chrono::SecondsFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// No fraction.
    Seconds,
    /// 3 digits.
    Millis,
    /// 6 digits.
    Micros,
    /// 9 digits.
    Nanos,
    /// None, or 3, 6, 9 digits depending on the value (`SecondsFormat::AutoSi`).
    Auto,
}

/// A string that cannot be read as a point in time or a date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    input: String,
    expected: &'static str,
}

impl ParseError {
    fn new(input: &str, expected: &'static str) -> Self {
        // Giữ tối đa 64 ký tự của đầu vào: đủ để biết sai ở đâu, không chép cả thân request vào log.
        ParseError { input: input.chars().take(64).collect(), expected }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not {}", self.input, self.expected)
    }
}

impl std::error::Error for ParseError {}
