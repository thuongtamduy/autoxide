//! So kiểu thời gian với `chrono` (thư viện cũ, đã gỡ): chuỗi lưu SQLite, JSON, cách đọc phải y hệt, để dữ liệu và
//! client cũ không thấy khác (D-20261121-time-library). Giá trị của `chrono` nằm ở `golden.json`, sinh MỘT lần bằng
//! chrono 0.4.45 + chrono-tz 0.10.4 + utoipa 6 (feature `chrono`) lúc còn dùng song song — không sửa tay.

use jiff::{SignedDuration, Unit};
use serde_json::Value;

use super::{Date, Precision, Timestamp};

/// Bảng đối chiếu sinh từ `chrono`.
fn golden() -> Value {
    serde_json::from_str(include_str!("golden.json")).unwrap()
}

fn rows(key: &str) -> Vec<Value> {
    golden()[key].as_array().unwrap().clone()
}

/// Thời điểm của một dòng `timestamps`: đúng giây, phần lẻ 1–9 chữ số, sát nửa đêm, năm nhuận, trước 1970, sát
/// giới hạn trên của `jiff` (9999-12-30T22:00Z).
fn timestamp_of(row: &Value) -> Timestamp {
    let nanos = u32::try_from(row["nanos"].as_u64().unwrap()).unwrap();
    Timestamp::from_unix(row["seconds"].as_i64().unwrap(), nanos).unwrap()
}

fn date_of(row: &Value) -> Date {
    let ymd = row["ymd"].as_array().unwrap();
    let part = |index: usize| ymd[index].as_i64().unwrap();
    Date::new(
        i16::try_from(part(0)).unwrap(),
        i8::try_from(part(1)).unwrap(),
        i8::try_from(part(2)).unwrap(),
    )
    .unwrap()
}

/// Chuỗi hay `null` của bảng → `Option<&str>`.
fn text(value: &Value) -> Option<&str> {
    value.as_str()
}

#[test]
fn chuoi_luu_sqlite_va_json_giong_het_chrono() {
    for row in rows("timestamps") {
        let ours = timestamp_of(&row);
        assert_eq!(ours.to_sql_text(), row["sqlite"].as_str().unwrap(), "{row}");
        assert_eq!(serde_json::to_value(ours).unwrap(), row["json"], "{row}");
    }
}

#[test]
fn chuoi_khac_va_so_giay_giong_chrono() {
    for row in rows("timestamps") {
        let ours = timestamp_of(&row);
        assert_eq!(ours.to_rfc3339(), row["rfc3339"].as_str().unwrap(), "{row}");
        assert_eq!(ours.to_string(), row["display"].as_str().unwrap(), "{row}");
        assert_eq!(format!("{ours:?}"), row["debug"].as_str().unwrap(), "{row}");
        assert_eq!(
            ours.to_rfc3339_opts(Precision::Micros, true),
            row["micros_z"].as_str().unwrap()
        );
        assert_eq!(
            ours.to_rfc3339_opts(Precision::Seconds, false),
            row["secs_offset"].as_str().unwrap()
        );
        for (pattern, expected) in row["formats"].as_object().unwrap() {
            assert_eq!(ours.format(pattern), expected.as_str().unwrap(), "{pattern} {row}");
        }
        assert_eq!(ours.date_utc().to_string(), row["date"].as_str().unwrap());
        // `chrono` làm tròn XUỐNG trước 1970 (-0,5 giây là -1 giây), `jiff` cắt về 0: kernel theo `chrono`.
        assert_eq!(ours.as_unix_seconds(), row["unix_seconds"].as_i64().unwrap(), "{row}");
        assert_eq!(ours.as_unix_millis(), row["unix_millis"].as_i64().unwrap(), "{row}");
        assert_eq!(ours.as_unix_micros(), row["unix_micros"].as_i64().unwrap(), "{row}");
    }
}

#[test]
fn doc_json_nhu_chrono() {
    for row in rows("json_inputs") {
        let ours: Option<Timestamp> = serde_json::from_value(row["input"].clone()).ok();
        assert_eq!(ours.map(Timestamp::to_json_string).as_deref(), text(&row["utc"]), "{row}");
    }
}

#[test]
fn doc_rfc3339_chat_nhu_chrono() {
    for row in rows("rfc3339_inputs") {
        let ours = Timestamp::parse_rfc3339(row["input"].as_str().unwrap()).ok();
        assert_eq!(ours.map(Timestamp::to_json_string).as_deref(), text(&row["utc"]), "{row}");
    }
}

#[test]
fn ngay_giong_het_chrono() {
    for row in rows("dates") {
        let ours = date_of(&row);
        assert_eq!(ours.to_string(), row["display"].as_str().unwrap());
        assert_eq!(serde_json::to_value(ours).unwrap(), row["json"]);
        assert_eq!(
            i64::from(ours.weekday().to_monday_zero_offset()),
            row["weekday0"].as_i64().unwrap()
        );
        assert_eq!(i64::from(ours.ordinal()), row["ordinal"].as_i64().unwrap());
        for (months, expected) in row["add_months"].as_object().unwrap() {
            let months: i32 = months.parse().unwrap();
            let found = ours.checked_add_months(months).map(|day| day.to_string());
            assert_eq!(found.as_deref(), text(expected), "{row} {months}");
        }
        let show = |day: Option<Date>| day.map(|day| day.to_string());
        assert_eq!(show(ours.checked_add_days(40)).as_deref(), text(&row["add_days_40"]));
        assert_eq!(show(ours.succ()).as_deref(), text(&row["succ"]), "{row}");
        assert_eq!(show(ours.pred()).as_deref(), text(&row["pred"]), "{row}");
        assert_eq!(ours.format("%d/%m/%Y"), row["ddmmyyyy"].as_str().unwrap());
        assert_eq!(ours.start_of_day_utc().to_json_string(), row["start_of_day"].as_str().unwrap());
    }
    assert_eq!("2026-02-29".parse::<Date>().ok(), None);
    assert_eq!("2026-11-21T00:00:00".parse::<Date>().ok(), None);
    assert_eq!("2026-1-21".parse::<Date>().ok(), None);
    assert_eq!("2026-11-21".parse::<Date>().unwrap(), Date::new(2026, 11, 21).unwrap());
    let a = Date::new(2026, 3, 1).unwrap();
    assert_eq!(a.days_since(Date::new(2026, 2, 1).unwrap()), 28);
    assert_eq!(Date::new(2026, 2, 1).unwrap().days_since(a), -28);
    let vietnam = jiff::tz::TimeZone::get("Asia/Ho_Chi_Minh").unwrap();
    assert_eq!(a.start_of_day_in(&vietnam).unwrap().to_json_string(), "2026-02-28T17:00:00Z");
}

#[test]
fn ngay_theo_mau_va_gio_trong_ngay_doc_nhu_chrono() {
    for row in rows("date_patterns") {
        let found =
            Date::parse_from_str(row["input"].as_str().unwrap(), row["pattern"].as_str().unwrap());
        assert_eq!(found.ok().map(|day| day.to_string()).as_deref(), text(&row["date"]), "{row}");
    }
    for row in rows("clock") {
        let ours = super::parse_clock(row["input"].as_str().unwrap())
            .map(|time| Value::from(vec![i64::from(time.hour()), i64::from(time.minute())]));
        assert_eq!(ours.unwrap_or(Value::Null), row["hm"], "{row}");
    }
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

    assert_eq!(serde_json::to_value(New::schema()).unwrap(), golden()["openapi"]);
}

#[test]
fn gio_dia_phuong_doi_ra_thoi_diem_nhu_chrono_earliest() {
    // Giờ nhảy mùa hè: không có; giờ lặp lại: lần sớm hơn.
    let new_york = super::zone("America/New_York").unwrap();
    for row in rows("new_york_earliest") {
        let local = row["local"].as_str().unwrap().parse::<jiff::civil::DateTime>().unwrap();
        let ours = super::from_local(&new_york, local).map(Timestamp::to_json_string);
        assert_eq!(ours.as_deref(), text(&row["utc"]), "{row}");
    }
    let vietnam = super::zone("Asia/Ho_Chi_Minh").unwrap();
    let local = "2026-11-16T08:00".parse::<jiff::civil::DateTime>().unwrap();
    assert_eq!(
        super::from_local(&vietnam, local).map(Timestamp::to_json_string).as_deref(),
        Some("2026-11-16T01:00:00Z")
    );
}

#[test]
fn rfc3339_theo_gio_dia_phuong_nhu_chrono() {
    for row in rows("local_rfc3339") {
        let ours = Timestamp::from_unix_micros(row["micros"].as_i64().unwrap()).unwrap();
        for zone in ["Asia/Ho_Chi_Minh", "America/New_York", "UTC"] {
            let found = ours.to_rfc3339_in(&super::zone(zone).unwrap());
            assert_eq!(found, row[zone].as_str().unwrap(), "{zone} {row}");
        }
    }
}

#[test]
fn cong_thang_nhu_chrono() {
    for row in rows("add_months") {
        let from = Timestamp::parse_rfc3339(row["from"].as_str().unwrap()).unwrap();
        let months = i32::try_from(row["months"].as_i64().unwrap()).unwrap();
        let found = from.checked_add_months(months).map(Timestamp::to_rfc3339);
        assert_eq!(found.as_deref(), text(&row["result"]), "{row}");
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
