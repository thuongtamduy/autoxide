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

    /// Hôm nay theo UTC.
    pub fn today_utc() -> Self {
        super::Timestamp::now().date_utc()
    }

    /// Thứ trong tuần.
    pub fn weekday(self) -> jiff::civil::Weekday {
        self.0.weekday()
    }

    /// Ngày thứ mấy trong năm (1–366).
    pub fn ordinal(self) -> i16 {
        self.0.day_of_year()
    }

    /// Cộng `days` ngày (âm: lùi); `None` khi ra ngoài năm ±9999.
    pub fn checked_add_days(self, days: i64) -> Option<Self> {
        self.0.checked_add(jiff::Span::new().try_days(days).ok()?).ok().map(Date)
    }

    /// Lùi `days` ngày.
    pub fn checked_sub_days(self, days: i64) -> Option<Self> {
        self.checked_add_days(days.checked_neg()?)
    }

    /// Cộng `months` tháng (âm: lùi). Ngày không có ở tháng đích thì lấy ngày cuối tháng (31/1 + 1 tháng =
    /// 28 hay 29/2), như `chrono::NaiveDate::checked_add_months`.
    pub fn checked_add_months(self, months: i32) -> Option<Self> {
        self.0.checked_add(jiff::Span::new().try_months(months).ok()?).ok().map(Date)
    }

    /// Ngày hôm sau.
    pub fn succ(self) -> Option<Self> {
        self.0.tomorrow().ok().map(Date)
    }

    /// Ngày hôm trước.
    pub fn pred(self) -> Option<Self> {
        self.0.yesterday().ok().map(Date)
    }

    /// Số ngày từ `earlier` tới `self` (âm khi `earlier` muộn hơn).
    pub fn days_since(self, earlier: Date) -> i64 {
        self.0.duration_since(earlier.0).as_secs() / 86_400
    }

    /// Ngày đầu tháng.
    pub fn first_of_month(self) -> Self {
        Date(self.0.first_of_month())
    }

    /// Ngày cuối tháng.
    pub fn last_of_month(self) -> Self {
        Date(self.0.last_of_month())
    }

    /// 00:00 của ngày này theo UTC.
    pub fn start_of_day_utc(self) -> super::Timestamp {
        let at = self.0.to_zoned_datetime(jiff::tz::TimeZone::UTC).map(|zoned| zoned.timestamp());
        super::Timestamp::exact(at.unwrap_or(jiff::Timestamp::UNIX_EPOCH))
    }

    /// Thời điểm bắt đầu ngày này ở múi giờ `zone` (ngày đổi giờ mùa hè bắt đầu lúc 01:00 thì lấy lúc đó).
    pub fn start_of_day_in(self, zone: &jiff::tz::TimeZone) -> Option<super::Timestamp> {
        let zoned = self.0.to_zoned_datetime(zone.clone()).ok()?;
        zoned.start_of_day().ok().map(|start| super::Timestamp::exact(start.timestamp()))
    }

    /// Định dạng theo mẫu `strftime` (`%d/%m/%Y`, `%Y%m%d`…). Mẫu sai (lỗi của người viết code) cho chuỗi rỗng.
    pub fn format(self, pattern: &str) -> String {
        jiff::fmt::strtime::format(pattern, self.0).unwrap_or_default()
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
