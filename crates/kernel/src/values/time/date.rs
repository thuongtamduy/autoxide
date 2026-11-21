//! [`Date`]: một ngày theo lịch, không giờ, không múi giờ.

use std::fmt;
use std::str::FromStr;

use jiff::civil;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::ParseError;
use super::format;

/// Một ngày theo lịch (`2026-11-21`). Thay `chrono::NaiveDate` (D-20261121-time-library).
///
/// JSON và cột database (`schema::date`): `YYYY-MM-DD`, như trước. Tính lịch (thêm ngày, thứ, đầu tháng…)
/// qua [`Date::to_jiff`] rồi bọc lại bằng `From`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(civil::Date);

impl Date {
    /// Ngày từ năm, tháng (1–12), ngày (1–31); `None` khi không có ngày đó.
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
