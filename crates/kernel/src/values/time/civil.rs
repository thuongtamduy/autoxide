//! The civil calendar: [`TimeZone`], [`TimeOfDay`], [`Weekday`], [`DateTime`] (D-20261128-library-layer-plan
//! point 3). They replace `jiff::tz::TimeZone`, `jiff::civil::{Time, Weekday, DateTime}` in module code.

use std::fmt;

use jiff::tz::AmbiguousOffset;

use super::{Date, Timestamp};

/// An IANA time zone (`Asia/Ho_Chi_Minh`, `UTC`), from the time zone data packaged in the binary (it does not
/// depend on the server's own data).
#[derive(Clone, PartialEq, Eq)]
pub struct TimeZone(jiff::tz::TimeZone);

impl TimeZone {
    /// Coordinated Universal Time.
    pub const UTC: TimeZone = TimeZone(jiff::tz::TimeZone::UTC);

    /// The zone named `name`. The name must use the casing of the time zone database (`Asia/Ho_Chi_Minh`,
    /// not `asia/ho_chi_minh`): a stored name is read back and compared elsewhere, so only one form is accepted.
    pub fn get(name: &str) -> Option<Self> {
        let zone = jiff::tz::TimeZone::get(name).ok()?;
        (zone.iana_name() == Some(name)).then_some(TimeZone(zone))
    }

    /// The IANA name (`Asia/Ho_Chi_Minh`); `UTC` for [`TimeZone::UTC`].
    pub fn name(&self) -> &str {
        self.0.iana_name().unwrap_or("UTC")
    }

    /// The instant of local time `local` in this zone. A local time that does not exist (clocks jump
    /// forward) gives `None`; a local time that happens twice (clocks set back) takes the earlier instant.
    pub fn to_timestamp(&self, local: DateTime) -> Option<Timestamp> {
        let offset = match self.0.to_ambiguous_timestamp(local.0).offset() {
            AmbiguousOffset::Unambiguous { offset } => offset,
            // Hai lần cùng giờ: lần đầu còn mang độ lệch trước khi đổi giờ.
            AmbiguousOffset::Fold { before, .. } => before,
            AmbiguousOffset::Gap { .. } => return None,
        };
        offset.to_timestamp(local.0).ok().map(Timestamp::from)
    }
}

impl fmt::Debug for TimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Display for TimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A time zone a [`Timestamp`] or [`Date`] can be placed in: [`TimeZone`], and during the transition also
/// `jiff::tz::TimeZone` (old path, D-20261127-transition-paths). Sealed: only the kernel implements it.
pub trait Zone: sealed::Sealed {
    #[doc(hidden)]
    fn jiff_zone(&self) -> &jiff::tz::TimeZone;
}

impl Zone for TimeZone {
    fn jiff_zone(&self) -> &jiff::tz::TimeZone {
        &self.0
    }
}

impl Zone for jiff::tz::TimeZone {
    fn jiff_zone(&self) -> &jiff::tz::TimeZone {
        self
    }
}

mod sealed {
    // Trait niêm phong: crate ngoài thấy `Zone` nhưng không cài được cho kiểu khác, nên kernel đổi hay bỏ phần
    // cài cho kiểu của `jiff` mà không phá ai.
    pub trait Sealed {}
    impl Sealed for super::TimeZone {}
    impl Sealed for jiff::tz::TimeZone {}
}

/// A time of day without date or zone (`09:00`, `17:30:15`), like opening hours.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeOfDay(jiff::civil::Time);

impl TimeOfDay {
    /// 00:00:00.
    pub const MIDNIGHT: TimeOfDay = TimeOfDay(jiff::civil::Time::midnight());

    /// From hour (0–23), minute (0–59), second (0–59); `None` when out of range.
    pub fn new(hour: i8, minute: i8, second: i8) -> Option<Self> {
        jiff::civil::Time::new(hour, minute, second, 0).ok().map(TimeOfDay)
    }

    /// Parses `HH:MM` (`09:00`; `9:00` is also accepted). Seconds and `24:00` are not accepted.
    pub fn parse_hm(raw: &str) -> Option<Self> {
        jiff::civil::Time::strptime("%H:%M", raw).ok().map(TimeOfDay)
    }

    /// The hour, 0 to 23.
    pub fn hour(self) -> i8 {
        self.0.hour()
    }

    /// The minute, 0 to 59.
    pub fn minute(self) -> i8 {
        self.0.minute()
    }

    /// The second, 0 to 59.
    pub fn second(self) -> i8 {
        self.0.second()
    }

    /// `HH:MM` (`09:05`), the form [`TimeOfDay::parse_hm`] reads.
    pub fn to_hm(self) -> String {
        format!("{:02}:{:02}", self.0.hour(), self.0.minute())
    }
}

impl Default for TimeOfDay {
    fn default() -> Self {
        TimeOfDay::MIDNIGHT
    }
}

/// `HH:MM:SS` (`09:05:00`).
impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}:{:02}", self.0.hour(), self.0.minute(), self.0.second())
    }
}

impl fmt::Debug for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A day of the week, Monday first (ISO 8601).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    /// 1 for Monday … 7 for Sunday (ISO 8601).
    pub fn number_from_monday(self) -> i8 {
        match self {
            Weekday::Monday => 1,
            Weekday::Tuesday => 2,
            Weekday::Wednesday => 3,
            Weekday::Thursday => 4,
            Weekday::Friday => 5,
            Weekday::Saturday => 6,
            Weekday::Sunday => 7,
        }
    }

    /// The reverse of [`Weekday::number_from_monday`]; `None` outside 1–7.
    pub fn from_number_from_monday(number: i8) -> Option<Self> {
        Some(match number {
            1 => Weekday::Monday,
            2 => Weekday::Tuesday,
            3 => Weekday::Wednesday,
            4 => Weekday::Thursday,
            5 => Weekday::Friday,
            6 => Weekday::Saturday,
            7 => Weekday::Sunday,
            _ => return None,
        })
    }

    /// The following day (Sunday is followed by Monday).
    pub fn next(self) -> Self {
        Weekday::from_number_from_monday(self.number_from_monday() % 7 + 1)
            .unwrap_or(Weekday::Monday)
    }
}

// Đường cũ (D-20261127-transition-paths): đổi từ `Date::weekday()` cũ; gỡ cùng hàm đó.
impl From<jiff::civil::Weekday> for Weekday {
    fn from(value: jiff::civil::Weekday) -> Self {
        Weekday::from_number_from_monday(value.to_monday_one_offset()).unwrap_or(Weekday::Monday)
    }
}

/// A date and time of day without zone (`2026-11-28T09:00:00`): local time on a wall clock.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime(jiff::civil::DateTime);

impl DateTime {
    /// `date` at `time`.
    pub fn new(date: Date, time: TimeOfDay) -> Self {
        DateTime(jiff::civil::DateTime::from_parts(date.to_jiff(), time.0))
    }

    /// The calendar date.
    pub fn date(self) -> Date {
        Date::from(self.0.date())
    }

    /// The time of day.
    pub fn time(self) -> TimeOfDay {
        TimeOfDay(self.0.time())
    }

    /// The day of the week of the date.
    pub fn day_of_week(self) -> Weekday {
        Weekday::from(self.0.weekday())
    }

    pub(super) fn from_jiff(inner: jiff::civil::DateTime) -> Self {
        // Bỏ phần dưới giây: giờ trên đồng hồ của lịch chỉ tới giây.
        DateTime(inner.with().subsec_nanosecond(0).build().unwrap_or(inner))
    }
}

/// `YYYY-MM-DDTHH:MM:SS` (`2026-11-28T09:00:00`).
impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}T{}", self.date(), self.time())
    }
}

impl fmt::Debug for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mui_gio_theo_ten_iana_dung_chu_hoa() {
        let vn = TimeZone::get("Asia/Ho_Chi_Minh").unwrap();
        assert_eq!(vn.name(), "Asia/Ho_Chi_Minh");
        assert_eq!(vn.to_string(), "Asia/Ho_Chi_Minh");
        assert!(TimeZone::get("asia/ho_chi_minh").is_none());
        assert!(TimeZone::get("Nowhere/City").is_none());
        assert_eq!(TimeZone::UTC.name(), "UTC");
        assert_eq!(TimeZone::get("UTC").unwrap().name(), "UTC");
        assert_eq!(TimeZone::get("UTC").unwrap().name(), TimeZone::UTC.name());
    }

    #[test]
    fn gio_dia_phuong_doi_qua_lai_voi_timestamp() {
        let vn = TimeZone::get("Asia/Ho_Chi_Minh").unwrap();
        let at = Timestamp::from_utc(2026, 11, 28, 2, 30, 0).unwrap();
        let local = at.to_local(&vn);
        assert_eq!(local.to_string(), "2026-11-28T09:30:00");
        assert_eq!(local.day_of_week(), Weekday::Saturday);
        assert_eq!(local.time(), TimeOfDay::new(9, 30, 0).unwrap());
        assert_eq!(vn.to_timestamp(local), Some(at));
        assert_eq!(at.date_in(&vn), Date::new(2026, 11, 28).unwrap());
        assert_eq!(at.to_rfc3339_in(&vn), "2026-11-28T09:30:00+07:00");
        // Đường cũ cho cùng kết quả.
        assert_eq!(at.date_in(&super::super::zone("Asia/Ho_Chi_Minh").unwrap()), at.date_in(&vn));
    }

    #[test]
    fn gio_khong_ton_tai_va_gio_lap_lai() {
        let ny = TimeZone::get("America/New_York").unwrap();
        // 2026-03-08 02:30 không có (đồng hồ nhảy 02:00 → 03:00).
        let gap = Date::new(2026, 3, 8).unwrap().at(TimeOfDay::new(2, 30, 0).unwrap());
        assert_eq!(ny.to_timestamp(gap), None);
        // 2026-11-01 01:30 có hai lần: lấy lần đầu (còn giờ mùa hè, UTC-4).
        let fold = Date::new(2026, 11, 1).unwrap().at(TimeOfDay::new(1, 30, 0).unwrap());
        assert_eq!(ny.to_timestamp(fold), Timestamp::from_utc(2026, 11, 1, 5, 30, 0));
        assert_eq!(
            Date::new(2026, 3, 8).unwrap().start_of_day_in(&ny),
            Timestamp::from_utc(2026, 3, 8, 5, 0, 0)
        );
    }

    #[test]
    fn gio_trong_ngay() {
        let nine = TimeOfDay::parse_hm("9:05").unwrap();
        assert_eq!(nine.to_hm(), "09:05");
        assert_eq!(nine.to_string(), "09:05:00");
        assert_eq!((nine.hour(), nine.minute(), nine.second()), (9, 5, 0));
        assert!(TimeOfDay::parse_hm("24:00").is_none());
        assert!(TimeOfDay::parse_hm("09:00:00").is_none());
        assert!(TimeOfDay::new(23, 60, 0).is_none());
        assert!(TimeOfDay::MIDNIGHT < nine);
        assert_eq!(TimeOfDay::default(), TimeOfDay::MIDNIGHT);
    }

    #[test]
    fn thu_trong_tuan() {
        assert_eq!(Date::new(2026, 11, 30).unwrap().day_of_week(), Weekday::Monday);
        assert_eq!(Weekday::Sunday.number_from_monday(), 7);
        assert_eq!(Weekday::from_number_from_monday(3), Some(Weekday::Wednesday));
        assert_eq!(Weekday::from_number_from_monday(0), None);
        assert_eq!(Weekday::Sunday.next(), Weekday::Monday);
        assert_eq!(Weekday::Monday.next(), Weekday::Tuesday);
        let all = [
            jiff::civil::Weekday::Monday,
            jiff::civil::Weekday::Tuesday,
            jiff::civil::Weekday::Wednesday,
            jiff::civil::Weekday::Thursday,
            jiff::civil::Weekday::Friday,
            jiff::civil::Weekday::Saturday,
            jiff::civil::Weekday::Sunday,
        ];
        for (index, day) in all.into_iter().enumerate() {
            assert_eq!(Weekday::from(day).number_from_monday(), i8::try_from(index + 1).unwrap());
        }
    }

    #[test]
    fn ngay_gio_ghep_va_tach() {
        let date = Date::new(2026, 11, 28).unwrap();
        let time = TimeOfDay::new(17, 30, 15).unwrap();
        let local = DateTime::new(date, time);
        assert_eq!(local.date(), date);
        assert_eq!(local.time(), time);
        assert_eq!(date.at(time), local);
        assert!(date.at(TimeOfDay::MIDNIGHT) < local);
    }
}
