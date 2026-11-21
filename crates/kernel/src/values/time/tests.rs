//! So kiểu thời gian mới với `chrono` (bản đang được thay): chuỗi lưu SQLite, JSON, cách đọc phải y hệt,
//! để dữ liệu và client cũ không thấy khác (D-20261121-time-library).

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use jiff::{SignedDuration, Unit};

use super::{Date, Timestamp};

/// Các thời điểm khó: đúng giây, phần lẻ 1–9 chữ số, sát nửa đêm, năm nhuận, trước 1970, sát giới hạn trên của
/// `jiff` (9999-12-30T22:00Z — hẹp hơn `chrono` để mọi múi giờ vẫn biểu diễn được).
fn samples() -> Vec<DateTime<Utc>> {
    let nanos = [
        0,
        1,
        10,
        100_000,
        120_000_000,
        123_000_000,
        123_456_000,
        123_456_789,
        999_999_999,
        500_000_000,
    ];
    let seconds = [0_i64, 1_763_719_200, 951_782_399, 4_102_444_799, -1, -86_401, 253_402_000_000];
    let mut out = Vec::new();
    for second in seconds {
        for nano in nanos {
            out.push(DateTime::from_timestamp(second, nano).unwrap());
        }
    }
    out
}

#[test]
fn chuoi_luu_sqlite_giong_het_chrono() {
    for value in samples() {
        let ours = Timestamp::from(value).to_sql_text();
        assert_eq!(ours, value.to_rfc3339_opts(SecondsFormat::AutoSi, false), "{value:?}");
    }
}

#[test]
fn json_giong_het_chrono() {
    for value in samples() {
        let ours = serde_json::to_string(&Timestamp::from(value)).unwrap();
        assert_eq!(ours, serde_json::to_string(&value).unwrap(), "{value:?}");
    }
}

#[test]
fn doc_json_nhu_chrono() {
    let accepted = [
        "2026-11-21T10:00:00Z",
        "2026-11-21T10:00:00.5Z",
        "2026-11-21T17:00:00+07:00",
        "2026-11-21T10:00:00.123456789-03:30",
        "2026-11-21t10:00:00z",
    ];
    for input in accepted {
        let json = format!("\"{input}\"");
        let ours: Timestamp = serde_json::from_str(&json).unwrap();
        let theirs: DateTime<Utc> = serde_json::from_str(&json).unwrap();
        assert_eq!(DateTime::<Utc>::from(ours), theirs, "{input}");
    }
    // Thiếu độ lệch giờ hay sai dạng: cả hai cùng từ chối.
    for input in ["2026-11-21T10:00:00", "2026-11-21", "hôm qua", ""] {
        let json = format!("\"{input}\"");
        assert!(serde_json::from_str::<Timestamp>(&json).is_err(), "{input}");
        assert!(serde_json::from_str::<DateTime<Utc>>(&json).is_err(), "{input}");
    }
}

#[test]
fn doc_moi_dang_sqlite_chrono_doc_duoc() {
    // Danh sách dạng `sqlx` + `chrono` nhận ở cột SQLite (sqlx-sqlite, `decode_datetime_from_text`).
    let cases = [
        ("2026-11-21T10:00:00.123456+00:00", "2026-11-21T10:00:00.123456Z"),
        ("2026-11-21T10:00:00+07:00", "2026-11-21T03:00:00Z"),
        ("2026-11-21 10:00:00", "2026-11-21T10:00:00Z"),
        ("2026-11-21 10:00:00.250", "2026-11-21T10:00:00.250Z"),
        ("2026-11-21 10:00", "2026-11-21T10:00:00Z"),
        ("2026-11-21 10:00Z", "2026-11-21T10:00:00Z"),
        ("2026-11-21 10:00+07:00", "2026-11-21T03:00:00Z"),
        ("2026-11-21 10:00:00.5Z", "2026-11-21T10:00:00.500Z"),
        ("2026-11-21 10:00:00.5+07:00", "2026-11-21T03:00:00.500Z"),
        ("2026-11-21T10:00", "2026-11-21T10:00:00Z"),
        ("2026-11-21T10:00Z", "2026-11-21T10:00:00Z"),
        ("2026-11-21T10:00:00", "2026-11-21T10:00:00Z"),
        ("2026-11-21T10:00:00.5Z", "2026-11-21T10:00:00.500Z"),
    ];
    for (stored, expected) in cases {
        let parsed = super::format::parse_lenient_timestamp(stored).map(Timestamp::exact);
        assert_eq!(parsed.map(|value| value.to_rfc3339()).as_deref(), Some(expected), "{stored}");
    }
    assert_eq!(super::format::parse_lenient_timestamp("không phải ngày"), None);
}

#[test]
fn cau_noi_chrono_di_mot_vong_khong_lech() {
    for value in samples() {
        let back: DateTime<Utc> = Timestamp::from(value).into();
        assert_eq!(back, value);
    }
}

#[test]
fn now_cat_ve_micro_giay() {
    let now = Timestamp::now();
    assert_eq!(now.to_jiff().subsec_nanosecond() % 1_000, 0);
    let from_jiff = Timestamp::from(jiff::Timestamp::new(10, 123_456_789).unwrap());
    assert_eq!(from_jiff.to_rfc3339(), "1970-01-01T00:00:10.123456Z");
}

#[test]
fn tinh_toan_co_ban() {
    let start: Timestamp = "2026-11-21T10:00:00Z".parse().unwrap();
    let later = start + SignedDuration::from_hours(25);
    assert_eq!(later.to_rfc3339(), "2026-11-22T11:00:00Z");
    assert_eq!(later - start, SignedDuration::from_hours(25));
    assert!(later > start);
    let truncated: Timestamp =
        "2026-11-21T10:59:59.999Z".parse::<Timestamp>().unwrap().truncate(Unit::Minute);
    assert_eq!(truncated.to_rfc3339(), "2026-11-21T10:59:00Z");
    let vietnam = jiff::tz::TimeZone::get("Asia/Ho_Chi_Minh").unwrap();
    let late_evening: Timestamp = "2026-11-21T18:30:00Z".parse().unwrap();
    assert_eq!(late_evening.date_in(&vietnam).to_string(), "2026-11-22");
}

#[test]
fn ngay_giong_het_chrono() {
    for (year, month, day) in
        [(2026, 11, 21), (2024, 2, 29), (1970, 1, 1), (1, 1, 1), (9999, 12, 31)]
    {
        let chrono_date = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        let ours = Date::from(chrono_date);
        assert_eq!(ours.to_string(), chrono_date.format("%F").to_string());
        assert_eq!(
            serde_json::to_string(&ours).unwrap(),
            serde_json::to_string(&chrono_date).unwrap()
        );
        assert_eq!(NaiveDate::from(ours), chrono_date);
    }
    assert_eq!("2026-02-29".parse::<Date>().ok(), None);
    assert_eq!("2026-11-21T00:00:00".parse::<Date>().ok(), None);
    assert_eq!("2026-1-21".parse::<Date>().ok(), None);
    assert_eq!("2026-11-21".parse::<Date>().unwrap(), Date::new(2026, 11, 21).unwrap());
}
