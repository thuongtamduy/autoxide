//! Viết và đọc chuỗi thời gian đúng dạng `chrono` đã dùng (JSON và cột SQLite), để đổi thư viện mà dữ liệu cũ,
//! dữ liệu mới và client không thấy khác.

use std::fmt::Write as _;

use jiff::civil;
use jiff::fmt::temporal::DateTimeParser;
use jiff::tz::TimeZone;

use super::Precision;

/// Đuôi của giờ UTC: JSON dùng `Z`, cột SQLite dùng `+00:00` (như `chrono` qua `sqlx` đã ghi).
#[derive(Clone, Copy)]
pub(super) enum UtcSuffix {
    Z,
    Offset,
}

// Bộ đọc dùng chung. `const fn new` nên dựng sẵn được lúc biên dịch, không tốn gì mỗi lần đọc.
static PARSER: DateTimeParser = DateTimeParser::new();

/// RFC 3339 theo UTC, phần lẻ của giây 0, 3, 6 hoặc 9 chữ số tuỳ giá trị (`SecondsFormat::AutoSi` của
/// `chrono`): `2026-11-21T10:00:00Z`, `…:00.120Z`, `…:00.123456Z`.
///
/// Vì sao không dùng `Display` của `jiff`: nó cắt số 0 cuối (`.12Z`), mà chuỗi như vậy so sánh sai thứ tự
/// (`.12Z` > `.125Z` vì `Z` > `5`) và khác chuỗi đã lưu.
pub(super) fn write_timestamp(timestamp: jiff::Timestamp, suffix: UtcSuffix) -> String {
    write_timestamp_with(timestamp, Precision::Auto, suffix)
}

/// Như [`write_timestamp`] với số chữ số lẻ chọn trước (`chrono::SecondsFormat`).
pub(super) fn write_timestamp_with(
    timestamp: jiff::Timestamp,
    precision: Precision,
    suffix: UtcSuffix,
) -> String {
    let datetime = timestamp.to_zoned_datetime(TimeZone::UTC).datetime();
    let mut out = String::with_capacity(32);
    write_date_parts(&mut out, datetime.year(), datetime.month(), datetime.day());
    // `write!` vào `String` không bao giờ lỗi; bỏ qua `Result` thay vì `unwrap` (cấm ngoài test).
    let _ =
        write!(out, "T{:02}:{:02}:{:02}", datetime.hour(), datetime.minute(), datetime.second());
    write_fraction(&mut out, datetime.subsec_nanosecond(), precision);
    out.push_str(match suffix {
        UtcSuffix::Z => "Z",
        UtcSuffix::Offset => "+00:00",
    });
    out
}

/// RFC 3339 theo giờ địa phương của `zone`, kèm độ lệch: `2026-11-23T09:00:00+07:00` — đúng chuỗi
/// `chrono::DateTime<Tz>::to_rfc3339()` (UTC cũng viết `+00:00`; độ lệch làm tròn tới phút như `chrono`).
pub(super) fn write_zoned(timestamp: jiff::Timestamp, zone: &TimeZone) -> String {
    let offset = zone.to_offset(timestamp);
    let datetime = offset.to_datetime(timestamp);
    let mut out = String::with_capacity(32);
    write_date_parts(&mut out, datetime.year(), datetime.month(), datetime.day());
    let _ =
        write!(out, "T{:02}:{:02}:{:02}", datetime.hour(), datetime.minute(), datetime.second());
    write_fraction(&mut out, datetime.subsec_nanosecond(), Precision::Auto);
    let seconds = offset.seconds();
    let minutes = (seconds.abs() + 30) / 60;
    let sign = if seconds < 0 { '-' } else { '+' };
    let _ = write!(out, "{sign}{:02}:{:02}", minutes / 60, minutes % 60);
    out
}

/// Phần lẻ của giây: `Auto` như `chrono` (`SecondsFormat::AutoSi`) — không có, hay 3, 6, 9 chữ số.
fn write_fraction(out: &mut String, nanos: i32, precision: Precision) {
    match precision {
        Precision::Seconds => {}
        Precision::Millis => {
            let _ = write!(out, ".{:03}", nanos / 1_000_000);
        }
        Precision::Micros => {
            let _ = write!(out, ".{:06}", nanos / 1_000);
        }
        Precision::Nanos => {
            let _ = write!(out, ".{nanos:09}");
        }
        Precision::Auto if nanos == 0 => {}
        Precision::Auto if nanos % 1_000_000 == 0 => {
            let _ = write!(out, ".{:03}", nanos / 1_000_000);
        }
        Precision::Auto if nanos % 1_000 == 0 => {
            let _ = write!(out, ".{:06}", nanos / 1_000);
        }
        Precision::Auto => {
            let _ = write!(out, ".{nanos:09}");
        }
    }
}

/// `2026-11-21 10:00:00.123456 UTC` — `Display` của `chrono::DateTime<Utc>`.
pub(super) fn write_display(timestamp: jiff::Timestamp) -> String {
    let datetime = timestamp.to_zoned_datetime(TimeZone::UTC).datetime();
    let mut out = String::with_capacity(32);
    write_date_parts(&mut out, datetime.year(), datetime.month(), datetime.day());
    let _ =
        write!(out, " {:02}:{:02}:{:02}", datetime.hour(), datetime.minute(), datetime.second());
    write_fraction(&mut out, datetime.subsec_nanosecond(), Precision::Auto);
    out.push_str(" UTC");
    out
}

/// `YYYY-MM-DD` (năm ngoài 0…9999 có dấu, như `chrono`).
pub(super) fn write_date(date: civil::Date) -> String {
    let mut out = String::with_capacity(10);
    write_date_parts(&mut out, date.year(), date.month(), date.day());
    out
}

fn write_date_parts(out: &mut String, year: i16, month: i8, day: i8) {
    if (0..=9999).contains(&year) {
        let _ = write!(out, "{year:04}-{month:02}-{day:02}");
    } else {
        let _ = write!(out, "{year:+05}-{month:02}-{day:02}");
    }
}

/// Thời điểm có độ lệch giờ (`Z`, `+07:00`, `+07`) theo RFC 3339 / ISO 8601 — dạng client gửi lên.
pub(super) fn parse_timestamp(input: &str) -> Option<jiff::Timestamp> {
    PARSER.parse_timestamp(input).ok()
}

/// Như [`parse_timestamp`], và nhận cả chuỗi KHÔNG có độ lệch (hiểu là UTC): những dạng `sqlx` + `chrono` đã
/// đọc được từ cột SQLite (`2026-11-21 10:00:00`, `2026-11-21T10:00`, …), kể cả giá trị do chính SQLite ghi
/// (`CURRENT_TIMESTAMP`).
pub(super) fn parse_lenient_timestamp(input: &str) -> Option<jiff::Timestamp> {
    if let Some(timestamp) = parse_timestamp(input) {
        return Some(timestamp);
    }
    let datetime = PARSER.parse_datetime(input).ok()?;
    datetime.to_zoned_datetime(TimeZone::UTC).ok().map(|zoned| zoned.timestamp())
}

/// Đúng `YYYY-MM-DD` — không nhận chuỗi có giờ (`chrono::NaiveDate` cũng không nhận).
pub(super) fn parse_date(input: &str) -> Option<civil::Date> {
    let bytes = input.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes.iter().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if !shaped {
        return None;
    }
    PARSER.parse_date(input).ok()
}

/// Đúng dạng `chrono::DateTime::parse_from_rfc3339` nhận: `YYYY-MM-DD` + `T`/`t`/dấu cách + `HH:MM:SS`, phần lẻ
/// tuỳ chọn (ít nhất 1 chữ số), rồi `Z`/`z` hay `±HH:MM`.
pub(super) fn is_rfc3339(input: &str) -> bool {
    let bytes = input.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        bytes.get(range).is_some_and(|part| part.iter().all(u8::is_ascii_digit))
    };
    let at = |index: usize, wanted: &[u8]| bytes.get(index).is_some_and(|b| wanted.contains(b));
    let head = digits(0..4)
        && at(4, b"-")
        && digits(5..7)
        && at(7, b"-")
        && digits(8..10)
        && at(10, b"Tt ")
        && digits(11..13)
        && at(13, b":")
        && digits(14..16)
        && at(16, b":")
        && digits(17..19);
    if !head {
        return false;
    }
    let mut index = 19;
    if at(index, b".") {
        let start = index + 1;
        index = start;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    match bytes.get(index) {
        Some(b'Z' | b'z') => index + 1 == bytes.len(),
        Some(b'+' | b'-') => {
            index + 6 == bytes.len()
                && digits(index + 1..index + 3)
                && at(index + 3, b":")
                && digits(index + 4..index + 6)
        }
        _ => false,
    }
}
