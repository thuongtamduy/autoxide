//! [`Date`]: a calendar date, with no time and no time zone.

use std::fmt;
use std::str::FromStr;

use jiff::civil;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::ParseError;
use super::format;

/// A calendar date (`2026-11-21`). Replaces `chrono::NaiveDate` (D-20261121-time-library).
///
/// JSON and database column (`schema::date`): `YYYY-MM-DD`, as before. Calendar arithmetic (add days,
/// weekday, start of month…) goes through [`Date::to_jiff`] and wraps back with `From`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(civil::Date);

impl Date {
    /// A date from year, month (1–12), day (1–31); `None` when no such date exists.
    pub fn new(year: i16, month: i8, day: i8) -> Option<Self> {
        civil::Date::new(year, month, day).ok().map(Date)
    }

    pub fn to_jiff(self) -> civil::Date {
        self.0
    }

    pub fn year(self) -> i16 {
        self.0.year()
    }

    pub fn month(self) -> i8 {
        self.0.month()
    }

    pub fn day(self) -> i8 {
        self.0.day()
    }

    /// Parses with a `strptime` pattern (`%Y-%m-%d`, `%d/%m/%Y`…) like `chrono::NaiveDate::parse_from_str`.
    pub fn parse_from_str(input: &str, pattern: &str) -> Result<Self, ParseError> {
        civil::Date::strptime(pattern, input)
            .map(Date)
            .map_err(|_| ParseError::new(input, "a date in the expected format"))
    }

    /// Today in UTC.
    pub fn today_utc() -> Self {
        super::Timestamp::now().date_utc()
    }

    /// Day of the week.
    pub fn weekday(self) -> jiff::civil::Weekday {
        self.0.weekday()
    }

    /// Day of the year (1–366).
    pub fn ordinal(self) -> i16 {
        self.0.day_of_year()
    }

    /// Adds `days` days (negative: subtract); `None` when it falls outside year ±9999.
    pub fn checked_add_days(self, days: i64) -> Option<Self> {
        self.0.checked_add(jiff::Span::new().try_days(days).ok()?).ok().map(Date)
    }

    /// Subtracts `days` days.
    pub fn checked_sub_days(self, days: i64) -> Option<Self> {
        self.checked_add_days(days.checked_neg()?)
    }

    /// Adds `months` months (negative: subtract). A day that does not exist in the target month becomes the
    /// last day of the month (31/1 + 1 month = 28 or 29/2), like `chrono::NaiveDate::checked_add_months`.
    pub fn checked_add_months(self, months: i32) -> Option<Self> {
        self.0.checked_add(jiff::Span::new().try_months(months).ok()?).ok().map(Date)
    }

    /// The next day.
    pub fn succ(self) -> Option<Self> {
        self.0.tomorrow().ok().map(Date)
    }

    /// The previous day.
    pub fn pred(self) -> Option<Self> {
        self.0.yesterday().ok().map(Date)
    }

    /// Number of days from `earlier` to `self` (negative when `earlier` is later).
    pub fn days_since(self, earlier: Date) -> i64 {
        self.0.duration_since(earlier.0).as_secs() / 86_400
    }

    /// First day of the month.
    pub fn first_of_month(self) -> Self {
        Date(self.0.first_of_month())
    }

    /// Last day of the month.
    pub fn last_of_month(self) -> Self {
        Date(self.0.last_of_month())
    }

    /// 00:00 of this date in UTC. Dates at the calendar limits (9999-12-31, -9999-01-01) have no
    /// representable instant: clamped to [`Timestamp::MAX`](super::Timestamp::MAX) /
    /// [`MIN`](super::Timestamp::MIN), so "before / after this date" comparisons still go the right way.
    pub fn start_of_day_utc(self) -> super::Timestamp {
        match self.0.to_zoned_datetime(jiff::tz::TimeZone::UTC) {
            Ok(zoned) => super::Timestamp::exact(zoned.timestamp()),
            Err(_) if self.0.year() > 0 => super::Timestamp::MAX,
            Err(_) => super::Timestamp::MIN,
        }
    }

    /// The instant this date starts in time zone `zone` (a day where daylight saving starts at 01:00 gives
    /// that time).
    pub fn start_of_day_in(self, zone: &jiff::tz::TimeZone) -> Option<super::Timestamp> {
        let zoned = self.0.to_zoned_datetime(zone.clone()).ok()?;
        zoned.start_of_day().ok().map(|start| super::Timestamp::exact(start.timestamp()))
    }

    /// Formats with a `strftime` pattern (`%d/%m/%Y`, `%Y%m%d`…). A bad pattern (a coding error) gives an
    /// empty string.
    pub fn format(self, pattern: &str) -> String {
        jiff::fmt::strtime::format(pattern, self.0).unwrap_or_default()
    }
}

/// Defaults to 1970-01-01, like `chrono::NaiveDate`.
impl Default for Date {
    fn default() -> Self {
        Date(civil::Date::constant(1970, 1, 1))
    }
}

impl From<civil::Date> for Date {
    fn from(value: civil::Date) -> Self {
        Date(value)
    }
}

impl From<Date> for civil::Date {
    fn from(value: Date) -> Self {
        value.0
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format::write_date(self.0))
    }
}

impl fmt::Debug for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format::write_date(self.0))
    }
}

impl FromStr for Date {
    type Err = ParseError;

    fn from_str(input: &str) -> Result<Self, ParseError> {
        format::parse_date(input)
            .map(Date)
            .ok_or_else(|| ParseError::new(input, "a date (YYYY-MM-DD)"))
    }
}

impl Serialize for Date {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}
