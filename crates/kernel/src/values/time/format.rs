//! Writing and reading time strings in exactly the form `chrono` used (JSON and SQLite columns), so that
//! after switching libraries old data, new data and clients see no difference.

use std::fmt::Write as _;

use jiff::civil;
use jiff::fmt::temporal::DateTimeParser;
use jiff::tz::TimeZone;

use super::Precision;

/// Suffix of UTC time: JSON uses `Z`, SQLite columns use `+00:00` (as `chrono` through `sqlx` wrote).
#[derive(Clone, Copy)]
pub(super) enum UtcSuffix {
    Z,
    Offset,
}

// Bộ đọc dùng chung. `const fn new` nên dựng sẵn được lúc biên dịch, không tốn gì mỗi lần đọc.
static PARSER: DateTimeParser = DateTimeParser::new();

/// RFC 3339 in UTC, fractional seconds of 0, 3, 6 or 9 digits depending on the value (`SecondsFormat::AutoSi`
/// of `chrono`): `2026-11-21T10:00:00Z`, `…:00.120Z`, `…:00.123456Z`.
///
/// Why `jiff`'s `Display` is not used: it cuts trailing zeros (`.12Z`), and such strings compare in the wrong
/// order (`.12Z` > `.125Z` because `Z` > `5`) and differ from the stored strings.
pub(super) fn write_timestamp(timestamp: jiff::Timestamp, suffix: UtcSuffix) -> String {
    write_timestamp_with(timestamp, Precision::Auto, suffix)
}

/// Like [`write_timestamp`] with the number of fractional digits chosen in advance (`chrono::SecondsFormat`).
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

/// RFC 3339 in `zone`'s local time, with the offset: `2026-11-23T09:00:00+07:00` — exactly the string of
/// `chrono::DateTime<Tz>::to_rfc3339()` (UTC is also written `+00:00`; the offset is rounded to minutes like
/// `chrono`).
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

/// Fractional seconds: `Auto` like `chrono` (`SecondsFormat::AutoSi`) — none, or 3, 6, 9 digits.
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

/// `2026-11-21 10:00:00.123456 UTC` — `Display` of `chrono::DateTime<Utc>`.
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

/// `YYYY-MM-DD` (a year outside 0…9999 is signed, like `chrono`).
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

/// A point in time with a UTC offset (`Z`, `+07:00`, `+07`) per RFC 3339 / ISO 8601 — the form clients send.
pub(super) fn parse_timestamp(input: &str) -> Option<jiff::Timestamp> {
    PARSER.parse_timestamp(input).ok()
}

/// Like [`parse_timestamp`], and also accepts strings WITHOUT an offset (taken as UTC): the forms `sqlx` +
/// `chrono` could read from SQLite columns (`2026-11-21 10:00:00`, `2026-11-21T10:00`, …), including values
/// written by SQLite itself (`CURRENT_TIMESTAMP`).
pub(super) fn parse_lenient_timestamp(input: &str) -> Option<jiff::Timestamp> {
    if let Some(timestamp) = parse_timestamp(input) {
        return Some(timestamp);
    }
    let datetime = PARSER.parse_datetime(input).ok()?;
    datetime.to_zoned_datetime(TimeZone::UTC).ok().map(|zoned| zoned.timestamp())
}

/// Exactly `YYYY-MM-DD` — strings with a time are not accepted (`chrono::NaiveDate` does not accept them either).
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

/// Exactly the form `chrono::DateTime::parse_from_rfc3339` accepts: `YYYY-MM-DD` + `T`/`t`/space +
/// `HH:MM:SS`, an optional fraction (at least 1 digit), then `Z`/`z` or `±HH:MM`.
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
