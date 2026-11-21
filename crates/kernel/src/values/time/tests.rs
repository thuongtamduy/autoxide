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
        assert_eq!(
            parsed.map(|value| value.to_json_string()).as_deref(),
            Some(expected),
            "{stored}"
        );
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
    assert_eq!(from_jiff.to_json_string(), "1970-01-01T00:00:10.123456Z");
}

#[test]
fn tinh_toan_co_ban() {
    let start: Timestamp = "2026-11-21T10:00:00Z".parse().unwrap();
    let later = start + SignedDuration::from_hours(25);
    assert_eq!(later.to_json_string(), "2026-11-22T11:00:00Z");
    assert_eq!(later - start, SignedDuration::from_hours(25));
    assert!(later > start);
    let truncated: Timestamp =
        "2026-11-21T10:59:59.999Z".parse::<Timestamp>().unwrap().truncate(Unit::Minute);
    assert_eq!(truncated.to_json_string(), "2026-11-21T10:59:00Z");
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

#[test]
fn openapi_giong_het_chrono() {
    use utoipa::PartialSchema as _;

    // Trường trong struct: `utoipa` nhận kiểu theo tên, phải ra đúng schema như kiểu `chrono` cũ.
    #[derive(utoipa::ToSchema)]
    #[allow(dead_code)]
    struct New {
        at: Timestamp,
        maybe_at: Option<Timestamp>,
        day: Date,
        days: Vec<Date>,
    }

    #[derive(utoipa::ToSchema)]
    #[allow(dead_code)]
    struct Old {
        at: DateTime<Utc>,
        maybe_at: Option<DateTime<Utc>>,
        day: NaiveDate,
        days: Vec<NaiveDate>,
    }

    let new = serde_json::to_value(New::schema()).unwrap();
    let old = serde_json::to_value(Old::schema()).unwrap();
    assert_eq!(new, old);
    assert_eq!(new["properties"]["at"]["format"], "date-time");
    assert_eq!(new["properties"]["day"]["format"], "date");
}

#[test]
fn chuoi_khac_giong_chrono() {
    use super::Precision;
    for value in samples() {
        let ours = Timestamp::from(value);
        assert_eq!(ours.to_rfc3339(), value.to_rfc3339(), "{value:?}");
        assert_eq!(ours.to_string(), value.to_string(), "{value:?}");
        assert_eq!(format!("{ours:?}"), format!("{value:?}"), "{value:?}");
        assert_eq!(
            ours.to_rfc3339_opts(Precision::Micros, true),
            value.to_rfc3339_opts(SecondsFormat::Micros, true)
        );
        assert_eq!(
            ours.to_rfc3339_opts(Precision::Seconds, false),
            value.to_rfc3339_opts(SecondsFormat::Secs, false)
        );
        for pattern in
            ["%Y-%m-%d", "%Y%m%dT%H%M%SZ", "%a, %d %b %Y %H:%M:%S GMT", "%H:%M %d/%m/%Y", "%F"]
        {
            assert_eq!(
                ours.format(pattern),
                value.format(pattern).to_string(),
                "{pattern} {value:?}"
            );
        }
        assert_eq!(NaiveDate::from(ours.date_utc()), value.date_naive());
    }
}

#[test]
fn tinh_ngay_giong_chrono() {
    use chrono::{Datelike as _, Months};
    for (year, month, day) in [(2024, 1, 31), (2024, 2, 29), (2026, 12, 31), (2026, 3, 1)] {
        let theirs = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        let ours = Date::from(theirs);
        assert_eq!(
            ours.weekday().to_monday_zero_offset(),
            theirs.weekday().num_days_from_monday() as i8
        );
        assert_eq!(i32::from(ours.ordinal()), theirs.ordinal() as i32);
        for months in [-13_i32, -1, 1, 2, 12] {
            let expected = if months >= 0 {
                theirs.checked_add_months(Months::new(months as u32))
            } else {
                theirs.checked_sub_months(Months::new(months.unsigned_abs()))
            };
            assert_eq!(
                ours.checked_add_months(months).map(NaiveDate::from),
                expected,
                "{theirs} {months}"
            );
        }
        assert_eq!(
            ours.checked_add_days(40).map(NaiveDate::from),
            theirs.checked_add_days(chrono::Days::new(40))
        );
        assert_eq!(ours.succ().map(NaiveDate::from), theirs.succ_opt());
        assert_eq!(ours.pred().map(NaiveDate::from), theirs.pred_opt());
        assert_eq!(ours.format("%d/%m/%Y"), theirs.format("%d/%m/%Y").to_string());
        assert_eq!(
            DateTime::<Utc>::from(ours.start_of_day_utc()),
            theirs.and_hms_opt(0, 0, 0).unwrap().and_utc()
        );
    }
    let a = Date::new(2026, 3, 1).unwrap();
    assert_eq!(a.days_since(Date::new(2026, 2, 1).unwrap()), 28);
    assert_eq!(Date::new(2026, 2, 1).unwrap().days_since(a), -28);
    let vietnam = jiff::tz::TimeZone::get("Asia/Ho_Chi_Minh").unwrap();
    assert_eq!(a.start_of_day_in(&vietnam).unwrap().to_json_string(), "2026-02-28T17:00:00Z");
}

#[test]
fn doc_rfc3339_chat_nhu_chrono_va_ngay_theo_mau() {
    for input in [
        "2026-11-21T10:00:00Z",
        "2026-11-21t10:00:00.5z",
        "2026-11-21 10:00:00+07:00",
        "2026-11-21T10:00:00.123456789-03:30",
        "2026-11-21T10:00Z",
        "2026-11-21T10:00:00",
        "2026-11-21",
        "20261121T100000Z",
        "2026-11-21T10:00:00+07",
        "2026-11-21T10:00:00.Z",
    ] {
        let chrono_ok = DateTime::parse_from_rfc3339(input).is_ok();
        assert_eq!(Timestamp::parse_rfc3339(input).is_ok(), chrono_ok, "{input}");
        if chrono_ok {
            let theirs = DateTime::parse_from_rfc3339(input).unwrap().with_timezone(&Utc);
            assert_eq!(DateTime::<Utc>::from(Timestamp::parse_rfc3339(input).unwrap()), theirs);
        }
    }
    for (input, pattern) in [
        ("2026-11-21", "%Y-%m-%d"),
        ("2026-1-5", "%Y-%m-%d"),
        ("21/11/2026", "%d/%m/%Y"),
        ("2026-13-01", "%Y-%m-%d"),
        ("x", "%Y-%m-%d"),
    ] {
        let theirs = NaiveDate::parse_from_str(input, pattern).ok();
        assert_eq!(
            Date::parse_from_str(input, pattern).ok().map(NaiveDate::from),
            theirs,
            "{input}"
        );
    }
}

#[test]
fn gio_trong_ngay_doc_nhu_chrono() {
    for raw in [
        "09:00", "9:00", "9:5", "23:59", " 09:00", "09:0", "24:00", "23:60", "09:00:00", "09:00 ",
        "0900",
    ] {
        let ours = super::parse_clock(raw).map(|time| (time.hour(), time.minute()));
        let theirs = chrono::NaiveTime::parse_from_str(raw, "%H:%M").ok().map(|time| {
            (
                i8::try_from(chrono::Timelike::hour(&time)).unwrap(),
                i8::try_from(chrono::Timelike::minute(&time)).unwrap(),
            )
        });
        assert_eq!(ours, theirs, "{raw}");
    }
}

#[test]
fn mui_gio_theo_ten_phan_biet_hoa_thuong_nhu_chrono_tz() {
    for name in ["Asia/Ho_Chi_Minh", "UTC", "Asia/Saigon", "Etc/GMT+7", "US/Pacific"] {
        assert_eq!(
            super::zone(name).and_then(|zone| zone.iana_name().map(str::to_owned)).as_deref(),
            Some(name)
        );
    }
    for name in ["asia/ho_chi_minh", "utc", "Local", "", "Asia/Nowhere"] {
        assert!(super::zone(name).is_none(), "{name}");
    }
}

#[test]
fn thoi_diem_theo_lich_utc() {
    let at = Timestamp::from_utc(2026, 11, 16, 18, 30, 5).unwrap();
    assert_eq!(at.to_json_string(), "2026-11-16T18:30:05Z");
    assert!(Timestamp::from_utc(2026, 2, 30, 0, 0, 0).is_none());
    assert!(Timestamp::from_utc(2026, 1, 1, 24, 0, 0).is_none());
}

#[test]
fn gio_dia_phuong_doi_ra_thoi_diem_nhu_chrono_earliest() {
    // Số đối chiếu lấy từ `chrono-tz` `from_local_datetime(…).earliest()`.
    let new_york = super::zone("America/New_York").unwrap();
    let local = |text: &str| text.parse::<jiff::civil::DateTime>().unwrap();
    let utc = |text: &str| Some(text.parse::<Timestamp>().unwrap());
    assert_eq!(super::from_local(&new_york, local("2026-03-08T02:30")), None, "nhảy giờ: không có");
    assert_eq!(
        super::from_local(&new_york, local("2026-03-08T03:00")),
        utc("2026-03-08T07:00:00Z")
    );
    assert_eq!(
        super::from_local(&new_york, local("2026-11-01T01:30")),
        utc("2026-11-01T05:30:00Z"),
        "lùi giờ: lần sớm"
    );
    assert_eq!(
        super::from_local(&new_york, local("2026-11-01T00:30")),
        utc("2026-11-01T04:30:00Z")
    );
    let vietnam = super::zone("Asia/Ho_Chi_Minh").unwrap();
    assert_eq!(super::from_local(&vietnam, local("2026-11-16T08:00")), utc("2026-11-16T01:00:00Z"));
}

#[test]
fn rfc3339_theo_gio_dia_phuong_nhu_chrono() {
    let vietnam = super::zone("Asia/Ho_Chi_Minh").unwrap();
    let new_york = super::zone("America/New_York").unwrap();
    for micros in [1_795_405_200_000_000_i64, 1_795_405_200_120_000, 1_795_405_200_123_456, 0, -1] {
        let ours = Timestamp::from_unix_micros(micros).unwrap();
        let theirs = DateTime::<Utc>::from_timestamp_micros(micros).unwrap();
        let offset = |hours: i32| chrono::FixedOffset::east_opt(hours * 3600).unwrap();
        assert_eq!(ours.to_rfc3339_in(&vietnam), theirs.with_timezone(&offset(7)).to_rfc3339());
        assert_eq!(ours.to_rfc3339_in(&jiff::tz::TimeZone::UTC), theirs.to_rfc3339());
        let new_york_hours =
            if new_york.to_offset(ours.to_jiff()).seconds() == -4 * 3600 { -4 } else { -5 };
        assert_eq!(
            ours.to_rfc3339_in(&new_york),
            theirs.with_timezone(&offset(new_york_hours)).to_rfc3339()
        );
    }
}

#[test]
fn cong_thang_nhu_chrono() {
    for (from, months) in [
        ("2026-01-31T10:20:30.123456Z", 1),
        ("2028-01-31T00:00:00Z", 1),
        ("2026-03-31T23:59:59Z", -1),
        ("2026-11-21T08:00:00Z", 12),
        ("2026-08-31T01:00:00Z", 18),
    ] {
        let ours = Timestamp::parse_rfc3339(from).unwrap().checked_add_months(months).unwrap();
        let start = DateTime::parse_from_rfc3339(from).unwrap().to_utc();
        let theirs = if months >= 0 {
            start.checked_add_months(chrono::Months::new(months.unsigned_abs())).unwrap()
        } else {
            start.checked_sub_months(chrono::Months::new(months.unsigned_abs())).unwrap()
        };
        assert_eq!(ours.to_rfc3339(), theirs.to_rfc3339(), "{from} {months}");
    }
}

#[test]
fn so_giay_truoc_1970_lam_tron_xuong_nhu_chrono() {
    let ours = Timestamp::from_unix(-1, 500_000_000).unwrap();
    let theirs = DateTime::<Utc>::from_timestamp(-1, 500_000_000).unwrap();
    assert_eq!(ours.as_unix_seconds(), theirs.timestamp());
    assert_eq!(ours.as_unix_millis(), theirs.timestamp_millis());
    assert_eq!(ours.as_unix_micros(), theirs.timestamp_micros());
    assert_eq!(ours.to_json_string(), theirs.to_rfc3339_opts(SecondsFormat::AutoSi, true));
}
