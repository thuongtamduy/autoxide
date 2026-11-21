//! [`Timestamp`]: một thời điểm, luôn hiểu theo UTC.

use std::fmt;
use std::ops::{Add, Sub};
use std::str::FromStr;

use jiff::tz::TimeZone;
use jiff::{RoundMode, SignedDuration, TimestampRound, Unit};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::format::{self, UtcSuffix};
use super::{Date, ParseError};

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

    /// Từ số giây kể từ 1970-01-01T00:00:00Z.
    pub fn from_unix_seconds(seconds: i64) -> Option<Self> {
        jiff::Timestamp::from_second(seconds).ok().map(Timestamp)
    }

    /// Số micro giây kể từ 1970-01-01T00:00:00Z (phần nhỏ hơn bị cắt).
    pub fn as_unix_micros(self) -> i64 {
        self.0.as_microsecond()
    }

    /// Số giây kể từ 1970-01-01T00:00:00Z (phần lẻ bị cắt về phía âm vô cùng như `jiff`).
    pub fn as_unix_seconds(self) -> i64 {
        self.0.as_second()
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

    /// RFC 3339 UTC như JSON trả ra: `2026-11-21T10:00:00.123456Z`.
    pub fn to_rfc3339(self) -> String {
        format::write_timestamp(self.0, UtcSuffix::Z)
    }

    /// Chuỗi lưu ở cột SQLite (`…+00:00`), cũng là chuỗi Postgres đọc vào `timestamptz`.
    pub(super) fn to_sql_text(self) -> String {
        format::write_timestamp(self.0, UtcSuffix::Offset)
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

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
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
        serializer.serialize_str(&self.to_rfc3339())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // `Cow` thay vì `&str`: chuỗi JSON có ký tự thoát không mượn được từ đầu vào.
        let text = std::borrow::Cow::<'de, str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}
