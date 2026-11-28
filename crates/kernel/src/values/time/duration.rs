//! [`Duration`]: a signed span of time with microsecond precision (D-20261128-library-layer-plan point 3).

use std::fmt;
use std::ops::{Add, Neg, Sub};

use super::Timestamp;

const MICROS_PER_MILLI: i64 = 1_000;
const MICROS_PER_SEC: i64 = 1_000_000;
const MICROS_PER_MIN: i64 = 60 * MICROS_PER_SEC;
const MICROS_PER_HOUR: i64 = 60 * MICROS_PER_MIN;
const MICROS_PER_DAY: i64 = 24 * MICROS_PER_HOUR;

/// A signed span of time, microsecond precision — the same precision as [`Timestamp`].
///
/// - Negative values are allowed: `later.since(earlier)` is negative when `earlier` is actually later.
/// - The constructors take whole units and saturate at [`Duration::MAX`] / [`Duration::MIN`] (about ±292,000
///   years) instead of overflowing; any such value is far outside what a [`Timestamp`] can hold, so adding
///   it to a timestamp fails in the checked form and clamps in the operator form (see below).
/// - Added to or subtracted from a [`Timestamp`]: [`Timestamp::checked_add`] / [`Timestamp::checked_sub`]
///   return `None` when the result leaves the representable range (years ±9999); the operators `+` / `-`
///   never panic and clamp to [`Timestamp::MIN`] / [`Timestamp::MAX`] instead. Use the checked form for
///   spans that come from user input.
/// - No `serde`: no API stores or returns a span; configuration reads whole numbers through `Setting`.
//
// Kiểu bọc một số `i64` micro giây, không bọc kiểu của `jiff`: hợp đồng công khai không lộ thư viện
// (D-20261126-library-isolation); đổi qua `jiff::SignedDuration` chỉ ở bên trong kernel.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Duration(i64);

impl Duration {
    /// No time at all.
    pub const ZERO: Duration = Duration(0);
    /// The longest positive span.
    pub const MAX: Duration = Duration(i64::MAX);
    /// The longest negative span.
    pub const MIN: Duration = Duration(i64::MIN);

    /// `micros` microseconds.
    pub const fn from_micros(micros: i64) -> Self {
        Duration(micros)
    }

    /// `millis` milliseconds.
    pub const fn from_millis(millis: i64) -> Self {
        Duration(millis.saturating_mul(MICROS_PER_MILLI))
    }

    /// `secs` seconds.
    pub const fn from_secs(secs: i64) -> Self {
        Duration(secs.saturating_mul(MICROS_PER_SEC))
    }

    /// `mins` minutes.
    pub const fn from_mins(mins: i64) -> Self {
        Duration(mins.saturating_mul(MICROS_PER_MIN))
    }

    /// `hours` hours.
    pub const fn from_hours(hours: i64) -> Self {
        Duration(hours.saturating_mul(MICROS_PER_HOUR))
    }

    /// `days` days of exactly 24 hours (not calendar days: a day where daylight saving changes is still 24 hours).
    pub const fn from_days(days: i64) -> Self {
        Duration(days.saturating_mul(MICROS_PER_DAY))
    }

    /// Whole microseconds.
    pub const fn as_micros(self) -> i64 {
        self.0
    }

    /// Whole milliseconds, truncated toward zero.
    pub const fn as_millis(self) -> i64 {
        self.0 / MICROS_PER_MILLI
    }

    /// Whole seconds, truncated toward zero.
    pub const fn as_secs(self) -> i64 {
        self.0 / MICROS_PER_SEC
    }

    /// Whole minutes, truncated toward zero.
    pub const fn as_mins(self) -> i64 {
        self.0 / MICROS_PER_MIN
    }

    /// Whole hours, truncated toward zero.
    pub const fn as_hours(self) -> i64 {
        self.0 / MICROS_PER_HOUR
    }

    /// Whole days of 24 hours, truncated toward zero.
    pub const fn as_days(self) -> i64 {
        self.0 / MICROS_PER_DAY
    }

    /// Whether the span is below zero.
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Whether the span is exactly zero.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// The span without its sign; [`Duration::MIN`] gives [`Duration::MAX`].
    pub const fn abs(self) -> Self {
        Duration(self.0.saturating_abs())
    }

    /// `self + other`; `None` on overflow.
    pub const fn checked_add(self, other: Duration) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(micros) => Some(Duration(micros)),
            None => None,
        }
    }

    /// `self - other`; `None` on overflow.
    pub const fn checked_sub(self, other: Duration) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(micros) => Some(Duration(micros)),
            None => None,
        }
    }

    /// `self * factor`; `None` on overflow.
    pub const fn checked_mul(self, factor: i64) -> Option<Self> {
        match self.0.checked_mul(factor) {
            Some(micros) => Some(Duration(micros)),
            None => None,
        }
    }

    /// The span as a standard library duration (for `tokio::time::sleep`, timeouts…); `None` when negative.
    pub fn to_std(self) -> Option<std::time::Duration> {
        u64::try_from(self.0).ok().map(std::time::Duration::from_micros)
    }

    /// From a standard library duration, truncated to microseconds; saturates at [`Duration::MAX`].
    pub fn from_std(duration: std::time::Duration) -> Self {
        Duration(i64::try_from(duration.as_micros()).unwrap_or(i64::MAX))
    }

    /// The same span for `jiff` (inside the kernel only).
    pub(crate) fn to_jiff(self) -> jiff::SignedDuration {
        jiff::SignedDuration::from_micros(self.0)
    }
}

// Đường cũ (D-20261127-transition-paths): hàm của kernel nhận `impl Into<Duration>` nên chỗ gọi cũ còn truyền
// `jiff::SignedDuration` vẫn biên dịch. Gỡ cùng bí danh `time::SignedDuration` khi module cuối đã chuyển.
impl From<jiff::SignedDuration> for Duration {
    /// Truncates below a microsecond; saturates outside about ±292,000 years.
    fn from(value: jiff::SignedDuration) -> Self {
        Duration(i64::try_from(value.as_micros()).unwrap_or(if value.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        }))
    }
}

impl Add for Duration {
    type Output = Duration;

    /// Saturates instead of overflowing.
    fn add(self, other: Duration) -> Duration {
        Duration(self.0.saturating_add(other.0))
    }
}

impl Sub for Duration {
    type Output = Duration;

    /// Saturates instead of overflowing.
    fn sub(self, other: Duration) -> Duration {
        Duration(self.0.saturating_sub(other.0))
    }
}

impl Neg for Duration {
    type Output = Duration;

    /// [`Duration::MIN`] gives [`Duration::MAX`].
    fn neg(self) -> Duration {
        Duration(self.0.saturating_neg())
    }
}

// Toán tử với `Timestamp` không bao giờ dừng chương trình: tràn thì kẹp về MIN / MAX (khoảng thời gian lấy từ
// cấu hình như "giữ 100000 ngày" vẫn cho mốc so sánh đúng chiều). Cần biết có tràn hay không thì dùng `checked_*`.
impl Add<Duration> for Timestamp {
    type Output = Timestamp;

    fn add(self, duration: Duration) -> Timestamp {
        self.checked_add(duration).unwrap_or(if duration.is_negative() {
            Timestamp::MIN
        } else {
            Timestamp::MAX
        })
    }
}

impl Sub<Duration> for Timestamp {
    type Output = Timestamp;

    fn sub(self, duration: Duration) -> Timestamp {
        self.checked_sub(duration).unwrap_or(if duration.is_negative() {
            Timestamp::MAX
        } else {
            Timestamp::MIN
        })
    }
}

/// `1h 30m`, `-45s`, `250ms`, `0s` — for messages and logs, not for parsing.
impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return f.write_str("0s");
        }
        if self.0 < 0 {
            f.write_str("-")?;
        }
        let mut rest = self.0.unsigned_abs();
        let mut first = true;
        for (unit, label) in [
            (MICROS_PER_DAY, "d"),
            (MICROS_PER_HOUR, "h"),
            (MICROS_PER_MIN, "m"),
            (MICROS_PER_SEC, "s"),
            (MICROS_PER_MILLI, "ms"),
            (1, "us"),
        ] {
            let unit = unit.unsigned_abs();
            let count = rest / unit;
            if count > 0 {
                if !first {
                    f.write_str(" ")?;
                }
                write!(f, "{count}{label}")?;
                first = false;
                rest %= unit;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dung_va_doi_don_vi() {
        assert_eq!(Duration::from_days(1), Duration::from_hours(24));
        assert_eq!(Duration::from_hours(1), Duration::from_mins(60));
        assert_eq!(Duration::from_mins(1), Duration::from_secs(60));
        assert_eq!(Duration::from_secs(1), Duration::from_millis(1_000));
        assert_eq!(Duration::from_millis(1), Duration::from_micros(1_000));
        assert_eq!(Duration::from_millis(1_999).as_secs(), 1);
        assert_eq!(Duration::from_millis(-1_999).as_secs(), -1);
        assert_eq!(Duration::from_hours(-36).as_days(), -1);
        assert_eq!(Duration::from_secs(90).as_millis(), 90_000);
        assert!(Duration::from_secs(-1).is_negative());
        assert!(Duration::ZERO.is_zero());
    }

    #[test]
    fn ham_dung_bao_hoa_khong_tran() {
        assert_eq!(Duration::from_days(i64::MAX), Duration::MAX);
        assert_eq!(Duration::from_days(i64::MIN), Duration::MIN);
        assert_eq!(Duration::MAX + Duration::from_secs(1), Duration::MAX);
        assert_eq!(-Duration::MIN, Duration::MAX);
        assert_eq!(Duration::MIN.abs(), Duration::MAX);
        assert_eq!(Duration::MAX.checked_add(Duration::from_micros(1)), None);
        assert_eq!(Duration::MIN.checked_sub(Duration::from_micros(1)), None);
        assert_eq!(Duration::MAX.checked_mul(2), None);
        assert_eq!(Duration::from_secs(2).checked_mul(3), Some(Duration::from_secs(6)));
    }

    #[test]
    fn cong_tru_voi_timestamp() {
        let at = Timestamp::from_utc(2026, 11, 28, 10, 0, 0).unwrap();
        let later = at + Duration::from_hours(2);
        assert_eq!(later, Timestamp::from_utc(2026, 11, 28, 12, 0, 0).unwrap());
        assert_eq!(later - Duration::from_hours(2), at);
        assert_eq!(at + Duration::from_hours(-2), at - Duration::from_hours(2));
        assert_eq!(later.since(at), Duration::from_hours(2));
        assert_eq!(at.since(later), Duration::from_hours(-2));
        assert_eq!(at.checked_add(Duration::from_millis(1)).unwrap().since(at).as_micros(), 1_000);
    }

    #[test]
    fn tran_timestamp_thi_loi_hoac_kep_khong_panic() {
        let at = Timestamp::from_utc(2026, 11, 28, 10, 0, 0).unwrap();
        assert_eq!(at.checked_add(Duration::from_days(10_000_000)), None);
        assert_eq!(at.checked_sub(Duration::from_days(10_000_000)), None);
        assert_eq!(at.checked_add(Duration::MAX), None);
        assert_eq!(at.checked_sub(Duration::MIN), None);
        assert_eq!(at + Duration::MAX, Timestamp::MAX);
        assert_eq!(at - Duration::MAX, Timestamp::MIN);
        assert_eq!(at + Duration::MIN, Timestamp::MIN);
        assert_eq!(at - Duration::MIN, Timestamp::MAX);
        assert!(Timestamp::MIN.since(Timestamp::MAX).is_negative());
    }

    #[test]
    fn duong_cu_signed_duration_van_dung() {
        let at = Timestamp::from_utc(2026, 11, 28, 10, 0, 0).unwrap();
        let old = jiff::SignedDuration::from_hours(3);
        assert_eq!(Duration::from(old), Duration::from_hours(3));
        assert_eq!(at.checked_add(old), Some(at + Duration::from_hours(3)));
        assert_eq!(Duration::from(jiff::SignedDuration::new(0, 1_999)).as_micros(), 1);
        assert_eq!(Duration::from(jiff::SignedDuration::MAX), Duration::MAX);
        assert_eq!(Duration::from(jiff::SignedDuration::MIN), Duration::MIN);
        assert_eq!(Duration::from_hours(5).to_jiff(), jiff::SignedDuration::from_hours(5));
    }

    #[test]
    fn doi_qua_std_va_hien_thi() {
        assert_eq!(Duration::from_secs(5).to_std(), Some(std::time::Duration::from_secs(5)));
        assert_eq!(Duration::from_secs(-5).to_std(), None);
        assert_eq!(
            Duration::from_std(std::time::Duration::from_nanos(1_500)),
            Duration::from_micros(1)
        );
        assert_eq!(Duration::from_std(std::time::Duration::MAX), Duration::MAX);
        assert_eq!(Duration::ZERO.to_string(), "0s");
        assert_eq!((Duration::from_hours(1) + Duration::from_mins(30)).to_string(), "1h 30m");
        assert_eq!(Duration::from_secs(-45).to_string(), "-45s");
        assert_eq!(Duration::from_millis(250).to_string(), "250ms");
        assert_eq!(Duration::from_micros(86_400_000_001).to_string(), "1d 1us");
    }
}
