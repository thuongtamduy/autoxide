//! [`Timestamp`]: an instant in time, always interpreted as UTC.

use std::fmt;
use std::ops::{Add, Sub};
use std::str::FromStr;

use jiff::tz::TimeZone;
use jiff::{RoundMode, SignedDuration, TimestampRound, Unit};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::format::{self, UtcSuffix};
use super::{Date, DateTime, Duration, ParseError, Precision, Zone};

/// An instant (UTC). Replaces `chrono::DateTime<Utc>` / `sea_orm::prelude::DateTimeUtc` (D-20261121-time-library).
///
/// - JSON: an RFC 3339 UTC string (`"2026-11-21T10:00:00.123456Z"`); accepts any offset (`+07:00`).
/// - Database column: `schema::timestamp_with_time_zone` — the storage form `chrono` wrote (see `time.rs`).
/// - Values created in the program ([`Timestamp::now`], `From<jiff::Timestamp>`) have microsecond precision,
///   same as Postgres. Values read from the database keep the stored precision (old SQLite data may have
///   nanoseconds): truncating them would make a `>` comparison against that very row wrong (pagination would
///   repeat the row).
//
// `#[derive(PartialOrd, Ord)]`: so sánh theo thời điểm (kiểu bọc chỉ có một trường).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(jiff::Timestamp);

impl Timestamp {
    /// 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: Timestamp = Timestamp(jiff::Timestamp::UNIX_EPOCH);
    /// The earliest representable instant (year −9999).
    pub const MIN: Timestamp = Timestamp(jiff::Timestamp::MIN);
    /// The latest representable instant (year 9999).
    pub const MAX: Timestamp = Timestamp(jiff::Timestamp::MAX);

    /// Now, truncated to microseconds.
    pub fn now() -> Self {
        Self::from(jiff::Timestamp::now())
    }

    /// Wraps keeping the precision unchanged — only for values read from storage (database, `chrono` bridge).
    pub(super) fn exact(inner: jiff::Timestamp) -> Self {
        Timestamp(inner)
    }

    /// The inner `jiff` value, for computation (time zones, rounding…).
    pub fn to_jiff(self) -> jiff::Timestamp {
        self.0
    }

    /// From microseconds since 1970-01-01T00:00:00Z. `None` when outside the range `jiff` can represent (year ±9999).
    pub fn from_unix_micros(micros: i64) -> Option<Self> {
        jiff::Timestamp::from_microsecond(micros).ok().map(Timestamp)
    }

    /// Adds `months` months in the UTC calendar, keeping the time of day; a day that does not exist in the
    /// target month becomes the last day of the month (31/01 + 1 month = 28/02) — like
    /// `chrono::DateTime<Utc>::checked_add_months`. `None` when outside the range.
    pub fn checked_add_months(self, months: i32) -> Option<Self> {
        let civil = jiff::tz::Offset::UTC.to_datetime(self.0);
        let later = civil.checked_add(jiff::Span::new().try_months(months).ok()?).ok()?;
        jiff::tz::Offset::UTC.to_timestamp(later).ok().map(Timestamp)
    }

    /// An instant from the UTC calendar (`2026-11-21 10:00:00`). `None` when the date and time do not exist.
    pub fn from_utc(
        year: i16,
        month: i8,
        day: i8,
        hour: i8,
        minute: i8,
        second: i8,
    ) -> Option<Self> {
        let civil = jiff::civil::DateTime::new(year, month, day, hour, minute, second, 0).ok()?;
        jiff::tz::Offset::UTC.to_timestamp(civil).ok().map(Timestamp)
    }

    /// From seconds and nanoseconds since 1970-01-01T00:00:00Z, keeping the precision unchanged (e.g. the
    /// instant inside a UUIDv7).
    pub fn from_unix(seconds: i64, nanos: u32) -> Option<Self> {
        let nanos = i32::try_from(nanos).ok()?;
        jiff::Timestamp::new(seconds, nanos).ok().map(Timestamp)
    }

    /// Parses RFC 3339 exactly like `chrono::DateTime::parse_from_rfc3339`: `YYYY-MM-DDTHH:MM:SS[.f]` then
    /// `Z` or `±HH:MM` (accepts `t`, `z` and a space in place of `T`). Stricter than [`FromStr`] — used where
    /// this was previously checked with `parse_from_rfc3339`.
    pub fn parse_rfc3339(input: &str) -> Result<Self, ParseError> {
        if !format::is_rfc3339(input) {
            return Err(ParseError::new(input, "an RFC 3339 date-time"));
        }
        input.parse()
    }

    /// From seconds since 1970-01-01T00:00:00Z.
    pub fn from_unix_seconds(seconds: i64) -> Option<Self> {
        jiff::Timestamp::from_second(seconds).ok().map(Timestamp)
    }

    /// Microseconds since 1970-01-01T00:00:00Z (the smaller part is truncated).
    pub fn as_unix_micros(self) -> i64 {
        floor_div(self.0, 1_000)
    }

    /// Seconds since 1970-01-01T00:00:00Z (the fractional part is truncated toward negative infinity, like `jiff`).
    pub fn as_unix_seconds(self) -> i64 {
        floor_div(self.0, 1_000_000_000)
    }

    /// Adds a span; `None` when the result leaves the representable range (years ±9999).
    //
    // `impl Into<Duration>`: chỗ gọi cũ còn truyền `jiff::SignedDuration` vẫn biên dịch (đường cũ).
    pub fn checked_add(self, duration: impl Into<Duration>) -> Option<Self> {
        self.0.checked_add(duration.into().to_jiff()).ok().map(Timestamp)
    }

    /// Subtracts a span; `None` when the result leaves the representable range (years ±9999).
    pub fn checked_sub(self, duration: impl Into<Duration>) -> Option<Self> {
        self.0.checked_sub(duration.into().to_jiff()).ok().map(Timestamp)
    }

    /// `self - earlier` as a [`Duration`], negative when `earlier` is later; below a microsecond is truncated.
    pub fn since(self, earlier: Timestamp) -> Duration {
        Duration::from(self.0.duration_since(earlier.0))
    }

    /// Old path: use [`Timestamp::since`]. `self - earlier` (negative when `earlier` is later).
    pub fn duration_since(self, earlier: Timestamp) -> SignedDuration {
        self.0.duration_since(earlier.0)
    }

    /// The local date and time of this instant in `zone` (below a second is dropped).
    pub fn to_local(self, zone: &super::TimeZone) -> DateTime {
        DateTime::from_jiff(self.0.to_zoned_datetime(zone.jiff_zone().clone()).datetime())
    }

    /// Truncates to unit `unit` (seconds, minutes…), without rounding up.
    pub fn truncate(self, unit: Unit) -> Self {
        let round = TimestampRound::new().smallest(unit).mode(RoundMode::Trunc);
        // Làm tròn chỉ lỗi khi `unit` lớn hơn giờ hay kết quả ra ngoài khoảng: giữ nguyên giá trị khi đó.
        self.0.round(round).map(Timestamp).unwrap_or(self)
    }

    /// The calendar date in time zone `zone` (e.g. the Vietnam date of an order).
    pub fn date_in(self, zone: &impl Zone) -> Date {
        Date::from(self.0.to_zoned_datetime(zone.jiff_zone().clone()).date())
    }

    /// RFC 3339 as JSON returns it: `2026-11-21T10:00:00.123456Z` (UTC, `Z` suffix, 0/3/6/9 fractional digits).
    pub fn to_json_string(self) -> String {
        format::write_timestamp(self.0, UtcSuffix::Z)
    }

    /// RFC 3339 in the local time of `zone`, with the offset (`2026-11-23T09:00:00+07:00`), like
    /// `chrono`'s `with_timezone(&tz).to_rfc3339()`: so a reader sees the time at that place.
    pub fn to_rfc3339_in(self, zone: &impl Zone) -> String {
        format::write_zoned(self.0, zone.jiff_zone())
    }

    /// RFC 3339 with the `+00:00` suffix — exactly the string `chrono::DateTime<Utc>::to_rfc3339()` produces
    /// (exports and strings already sent out earlier stay unchanged).
    pub fn to_rfc3339(self) -> String {
        format::write_timestamp(self.0, UtcSuffix::Offset)
    }

    /// RFC 3339 with a fixed number of fractional digits (`precision`) and the suffix `Z` (`use_z`) or
    /// `+00:00` — like `chrono::DateTime::to_rfc3339_opts`.
    pub fn to_rfc3339_opts(self, precision: Precision, use_z: bool) -> String {
        let suffix = if use_z { UtcSuffix::Z } else { UtcSuffix::Offset };
        format::write_timestamp_with(self.0, precision, suffix)
    }

    /// Formats with a `strftime` pattern (`%Y-%m-%d`, `%H:%M`, `%a, %d %b %Y %H:%M:%S GMT`…), in UTC. A bad
    /// pattern (a coding error) gives an empty string.
    pub fn format(self, pattern: &str) -> String {
        let zoned = self.0.to_zoned_datetime(TimeZone::UTC);
        jiff::fmt::strtime::format(pattern, &zoned).unwrap_or_default()
    }

    /// The calendar date in UTC.
    pub fn date_utc(self) -> Date {
        self.date_in(&super::TimeZone::UTC)
    }

    /// Old path: use [`Timestamp::to_local`]. This instant in time zone `zone` (to get local time, weekday…).
    pub fn in_zone(self, zone: &impl Zone) -> jiff::ZonedDateTime {
        self.0.to_zoned_datetime(zone.jiff_zone().clone())
    }

    /// From milliseconds since 1970-01-01T00:00:00Z.
    pub fn from_unix_millis(millis: i64) -> Option<Self> {
        jiff::Timestamp::from_millisecond(millis).ok().map(Timestamp)
    }

    /// Milliseconds since 1970-01-01T00:00:00Z (the smaller part is truncated).
    pub fn as_unix_millis(self) -> i64 {
        floor_div(self.0, 1_000_000)
    }

    /// The string stored in a SQLite column (`…+00:00`), also the string Postgres reads into `timestamptz`.
    pub(super) fn to_sql_text(self) -> String {
        format::write_timestamp(self.0, UtcSuffix::Offset)
    }
}

/// Defaults to 1970-01-01T00:00:00Z, like `chrono::DateTime<Utc>`.
impl Default for Timestamp {
    fn default() -> Self {
        Timestamp::UNIX_EPOCH
    }
}

impl From<jiff::Timestamp> for Timestamp {
    /// Truncates to microseconds (Postgres precision) — see the type's documentation.
    fn from(value: jiff::Timestamp) -> Self {
        Timestamp(value).truncate(Unit::Microsecond)
    }
}

impl From<Timestamp> for jiff::Timestamp {
    fn from(value: Timestamp) -> Self {
        value.0
    }
}

// Cộng / trừ bằng toán tử như `chrono` (`now + Duration::hours(1)`). Tràn (năm ±9999) thì `jiff` dừng chương
// trình như `chrono` trước đây; chỗ nhận khoảng do người dùng nhập thì dùng `checked_add`.
impl Add<SignedDuration> for Timestamp {
    type Output = Timestamp;

    fn add(self, duration: SignedDuration) -> Timestamp {
        Timestamp(self.0 + duration)
    }
}

impl Sub<SignedDuration> for Timestamp {
    type Output = Timestamp;

    fn sub(self, duration: SignedDuration) -> Timestamp {
        Timestamp(self.0 - duration)
    }
}

impl Sub for Timestamp {
    type Output = SignedDuration;

    fn sub(self, earlier: Timestamp) -> SignedDuration {
        self.duration_since(earlier)
    }
}

// `Display` như `chrono::DateTime<Utc>` (`2026-11-21 10:00:00.123456 UTC`): câu thông báo, log, tên file đã có
// từ trước giữ nguyên.
impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format::write_display(self.0))
    }
}

// `Debug` như `chrono` (RFC 3339, đuôi `Z`).
impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_json_string())
    }
}

impl FromStr for Timestamp {
    type Err = ParseError;

    /// RFC 3339 with a time offset (`Z`, `+07:00`); keeps the precision the sender supplied.
    fn from_str(input: &str) -> Result<Self, ParseError> {
        format::parse_timestamp(input)
            .map(Timestamp)
            .ok_or_else(|| ParseError::new(input, "an RFC 3339 date-time with offset"))
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_json_string())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // `Cow` thay vì `&str`: chuỗi JSON có ký tự thoát không mượn được từ đầu vào.
        let text = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Number of units (`unit` nanoseconds) since 1970, rounded DOWN like `chrono` (`timestamp()`,
/// `timestamp_millis()`…). `jiff` truncates toward 0, so before 1970 with a fractional part (-0.5 seconds)
/// the two differ: -1 second versus 0.
fn floor_div(at: jiff::Timestamp, unit: i128) -> i64 {
    // Mọi thời điểm `jiff` biểu diễn được (năm ±9999) đều vừa `i64` ở đơn vị micro giây trở lên.
    i64::try_from(at.as_nanosecond().div_euclid(unit)).unwrap_or(i64::MIN)
}
