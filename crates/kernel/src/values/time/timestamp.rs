//! [`Timestamp`]: một thời điểm, luôn hiểu theo UTC.

use std::fmt;
use std::ops::{Add, Sub};
use std::str::FromStr;

use jiff::tz::TimeZone;
use jiff::{RoundMode, SignedDuration, TimestampRound, Unit};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::format::{self, UtcSuffix};
use super::{Date, ParseError, Precision};

/// Một thời điểm (UTC). Thay `chrono::DateTime<Utc>` / `sea_orm::prelude::DateTimeUtc` (D-20261121-time-library).
///
/// - JSON: chuỗi RFC 3339 UTC (`"2026-11-21T10:00:00.123456Z"`); nhận vào mọi độ lệch giờ (`+07:00`).
/// - Cột database: `schema::timestamp_with_time_zone` — dạng lưu như `chrono` đã ghi (xem `time.rs`).
/// - Giá trị tạo trong chương trình ([`Timestamp::now`], `From<jiff::Timestamp>`) có độ chính xác micro giây,
///   bằng Postgres. Giá trị đọc từ database giữ nguyên độ chính xác đã lưu (SQLite cũ có thể có nano giây):
///   cắt bớt thì so sánh `>` với chính dòng đó sẽ sai (phân trang lặp lại dòng).
//
// `#[derive(PartialOrd, Ord)]`: so sánh theo thời điểm (kiểu bọc chỉ có một trường).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(jiff::Timestamp);

impl Timestamp {
    /// 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: Timestamp = Timestamp(jiff::Timestamp::UNIX_EPOCH);
    /// Thời điểm sớm nhất biểu diễn được (năm −9999).
    pub const MIN: Timestamp = Timestamp(jiff::Timestamp::MIN);
    /// Thời điểm muộn nhất biểu diễn được (năm 9999).
    pub const MAX: Timestamp = Timestamp(jiff::Timestamp::MAX);

    /// Bây giờ, cắt về micro giây.
    pub fn now() -> Self {
        Self::from(jiff::Timestamp::now())
    }

    /// Bọc giữ nguyên độ chính xác — chỉ cho giá trị đọc từ nơi lưu (database, cầu nối `chrono`).
    pub(super) fn exact(inner: jiff::Timestamp) -> Self {
        Timestamp(inner)
    }

    /// Giá trị `jiff` bên trong, để tính toán (múi giờ, làm tròn…).
    pub fn to_jiff(self) -> jiff::Timestamp {
        self.0
    }

    /// Từ số micro giây kể từ 1970-01-01T00:00:00Z. `None` khi ngoài khoảng `jiff` biểu diễn được (năm ±9999).
    pub fn from_unix_micros(micros: i64) -> Option<Self> {
        jiff::Timestamp::from_microsecond(micros).ok().map(Timestamp)
    }

    /// Cộng `months` tháng theo lịch UTC, giữ giờ trong ngày; ngày không có ở tháng đích lấy ngày cuối tháng
    /// (31/01 + 1 tháng = 28/02) — như `chrono::DateTime<Utc>::checked_add_months`. `None` khi ra ngoài khoảng.
    pub fn checked_add_months(self, months: i32) -> Option<Self> {
        let civil = jiff::tz::Offset::UTC.to_datetime(self.0);
        let later = civil.checked_add(jiff::Span::new().try_months(months).ok()?).ok()?;
        jiff::tz::Offset::UTC.to_timestamp(later).ok().map(Timestamp)
    }

    /// Thời điểm theo lịch UTC (`2026-11-21 10:00:00`). `None` khi ngày giờ không có thật.
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

    /// Từ giây và nano giây kể từ 1970-01-01T00:00:00Z, giữ nguyên độ chính xác (vd thời điểm trong UUIDv7).
    pub fn from_unix(seconds: i64, nanos: u32) -> Option<Self> {
        let nanos = i32::try_from(nanos).ok()?;
        jiff::Timestamp::new(seconds, nanos).ok().map(Timestamp)
    }

    /// Đọc RFC 3339 đúng như `chrono::DateTime::parse_from_rfc3339`: `YYYY-MM-DDTHH:MM:SS[.f]` rồi `Z` hay
    /// `±HH:MM` (chấp nhận `t`, `z` và dấu cách thay `T`). Chặt hơn [`FromStr`] — dùng ở chỗ trước đây kiểm
    /// bằng `parse_from_rfc3339`.
    pub fn parse_rfc3339(input: &str) -> Result<Self, ParseError> {
        if !format::is_rfc3339(input) {
            return Err(ParseError::new(input, "an RFC 3339 date-time"));
        }
        input.parse()
    }

    /// Từ số giây kể từ 1970-01-01T00:00:00Z.
    pub fn from_unix_seconds(seconds: i64) -> Option<Self> {
        jiff::Timestamp::from_second(seconds).ok().map(Timestamp)
    }

    /// Số micro giây kể từ 1970-01-01T00:00:00Z (phần nhỏ hơn bị cắt).
    pub fn as_unix_micros(self) -> i64 {
        floor_div(self.0, 1_000)
    }

    /// Số giây kể từ 1970-01-01T00:00:00Z (phần lẻ bị cắt về phía âm vô cùng như `jiff`).
    pub fn as_unix_seconds(self) -> i64 {
        floor_div(self.0, 1_000_000_000)
    }

    /// Cộng một khoảng; `None` khi vượt khoảng biểu diễn được.
    pub fn checked_add(self, duration: SignedDuration) -> Option<Self> {
        self.0.checked_add(duration).ok().map(Timestamp)
    }

    /// Trừ một khoảng; `None` khi vượt khoảng biểu diễn được.
    pub fn checked_sub(self, duration: SignedDuration) -> Option<Self> {
        self.0.checked_sub(duration).ok().map(Timestamp)
    }

    /// `self - earlier` (âm khi `earlier` muộn hơn).
    pub fn duration_since(self, earlier: Timestamp) -> SignedDuration {
        self.0.duration_since(earlier.0)
    }

    /// Cắt về đơn vị `unit` (giây, phút…), không làm tròn lên.
    pub fn truncate(self, unit: Unit) -> Self {
        let round = TimestampRound::new().smallest(unit).mode(RoundMode::Trunc);
        // Làm tròn chỉ lỗi khi `unit` lớn hơn giờ hay kết quả ra ngoài khoảng: giữ nguyên giá trị khi đó.
        self.0.round(round).map(Timestamp).unwrap_or(self)
    }

    /// Ngày theo lịch ở múi giờ `zone` (vd ngày ở Việt Nam của một đơn hàng).
    pub fn date_in(self, zone: &TimeZone) -> Date {
        Date::from(self.0.to_zoned_datetime(zone.clone()).date())
    }

    /// RFC 3339 như JSON trả ra: `2026-11-21T10:00:00.123456Z` (UTC, đuôi `Z`, phần lẻ 0/3/6/9 chữ số).
    pub fn to_json_string(self) -> String {
        format::write_timestamp(self.0, UtcSuffix::Z)
    }

    /// RFC 3339 theo giờ địa phương của `zone`, kèm độ lệch (`2026-11-23T09:00:00+07:00`), như
    /// `chrono` `with_timezone(&tz).to_rfc3339()`: cho người đọc thấy giờ ở nơi đó.
    pub fn to_rfc3339_in(self, zone: &TimeZone) -> String {
        format::write_zoned(self.0, zone)
    }

    /// RFC 3339 đuôi `+00:00` — đúng chuỗi `chrono::DateTime<Utc>::to_rfc3339()` cho ra (file xuất, chuỗi
    /// gửi ra ngoài đã có từ trước giữ nguyên).
    pub fn to_rfc3339(self) -> String {
        format::write_timestamp(self.0, UtcSuffix::Offset)
    }

    /// RFC 3339 với số chữ số lẻ cố định (`precision`) và đuôi `Z` (`use_z`) hay `+00:00` — như
    /// `chrono::DateTime::to_rfc3339_opts`.
    pub fn to_rfc3339_opts(self, precision: Precision, use_z: bool) -> String {
        let suffix = if use_z { UtcSuffix::Z } else { UtcSuffix::Offset };
        format::write_timestamp_with(self.0, precision, suffix)
    }

    /// Định dạng theo mẫu `strftime` (`%Y-%m-%d`, `%H:%M`, `%a, %d %b %Y %H:%M:%S GMT`…), giờ UTC. Mẫu sai
    /// (lỗi của người viết code) cho chuỗi rỗng.
    pub fn format(self, pattern: &str) -> String {
        let zoned = self.0.to_zoned_datetime(TimeZone::UTC);
        jiff::fmt::strtime::format(pattern, &zoned).unwrap_or_default()
    }

    /// Ngày theo lịch ở UTC.
    pub fn date_utc(self) -> Date {
        self.date_in(&TimeZone::UTC)
    }

    /// Thời điểm này ở múi giờ `zone` (để lấy giờ địa phương, thứ…).
    pub fn in_zone(self, zone: &TimeZone) -> jiff::ZonedDateTime {
        self.0.to_zoned_datetime(zone.clone())
    }

    /// Từ số mili giây kể từ 1970-01-01T00:00:00Z.
    pub fn from_unix_millis(millis: i64) -> Option<Self> {
        jiff::Timestamp::from_millisecond(millis).ok().map(Timestamp)
    }

    /// Số mili giây kể từ 1970-01-01T00:00:00Z (phần nhỏ hơn bị cắt).
    pub fn as_unix_millis(self) -> i64 {
        floor_div(self.0, 1_000_000)
    }

    /// Chuỗi lưu ở cột SQLite (`…+00:00`), cũng là chuỗi Postgres đọc vào `timestamptz`.
    pub(super) fn to_sql_text(self) -> String {
        format::write_timestamp(self.0, UtcSuffix::Offset)
    }
}

/// Mặc định 1970-01-01T00:00:00Z, như `chrono::DateTime<Utc>`.
impl Default for Timestamp {
    fn default() -> Self {
        Timestamp::UNIX_EPOCH
    }
}

impl From<jiff::Timestamp> for Timestamp {
    /// Cắt về micro giây (độ chính xác của Postgres) — xem tài liệu của kiểu.
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

    /// RFC 3339 có độ lệch giờ (`Z`, `+07:00`); giữ nguyên độ chính xác người gửi đưa.
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

/// Số đơn vị (`unit` nano giây) kể từ 1970, làm tròn XUỐNG như `chrono` (`timestamp()`, `timestamp_millis()`…).
/// `jiff` cắt về phía 0, nên trước 1970 có phần lẻ (-0,5 giây) hai bên khác nhau: -1 giây so với 0.
fn floor_div(at: jiff::Timestamp, unit: i128) -> i64 {
    // Mọi thời điểm `jiff` biểu diễn được (năm ±9999) đều vừa `i64` ở đơn vị micro giây trở lên.
    i64::try_from(at.as_nanosecond().div_euclid(unit)).unwrap_or(i64::MIN)
}
