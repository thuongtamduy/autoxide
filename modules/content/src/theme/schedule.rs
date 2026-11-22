//! Lịch của bản chủ đề (bước 67, D-234, D-20261122-theme-variants): `once` (khoảng UTC), `daily` (giờ địa
//! phương `HH:MM`, qua nửa đêm được, khoảng ngày tuỳ chọn), `weekly` (như `daily` kèm các thứ). Giờ địa
//! phương theo múi IANA của website — không cộng độ lệch cố định (giờ mùa hè). Hàm nhận `now` làm tham số
//! để test được; chỉ nhìn tối đa [`HORIZON_DAYS`] ngày tới khi tìm mốc đổi kế tiếp.

use autoxide_kernel::error::FieldError;
use autoxide_kernel::time::{self, Date, SignedDuration, Timestamp};
use jiff::civil::{Time, Weekday};
use jiff::tz::TimeZone;
use serde_json::{Map, Value, json};

/// Tìm mốc đổi chủ đề trong chừng này ngày tới (đủ cho mọi lịch `daily` / `weekly`).
pub(crate) const HORIZON_DAYS: i64 = 7;

/// Lịch đã kiểm.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Schedule {
    Once {
        starts_at: Timestamp,
        ends_at: Timestamp,
    },
    Repeat {
        start: Time,
        end: Time,
        days: Option<Vec<Weekday>>,
        from: Option<Date>,
        until: Option<Date>,
    },
}

/// Một lần chạy: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub start: Timestamp,
    pub end: Timestamp,
}

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("mon", Weekday::Monday),
    ("tue", Weekday::Tuesday),
    ("wed", Weekday::Wednesday),
    ("thu", Weekday::Thursday),
    ("fri", Weekday::Friday),
    ("sat", Weekday::Saturday),
    ("sun", Weekday::Sunday),
];

/// Kiểm `value` (JSON của lịch); mọi lỗi một lần, trường `schedule.<…>`.
pub(crate) fn check(value: &Value) -> Result<Schedule, Vec<FieldError>> {
    let mut errors = Vec::new();
    let Some(object) = value.as_object() else {
        return Err(vec![FieldError::new("schedule", "invalid", "must be a JSON object")]);
    };
    let kind = object.get("kind").and_then(Value::as_str).unwrap_or_default();
    let allowed: &[&str] = match kind {
        "once" => &["kind", "starts_at", "ends_at"],
        "daily" => &["kind", "start", "end", "from", "until"],
        "weekly" => &["kind", "start", "end", "days", "from", "until"],
        _ => {
            return Err(vec![FieldError::new(
                "schedule.kind",
                "invalid",
                "must be `once`, `daily` or `weekly`",
            )]);
        }
    };
    for key in object.keys().filter(|key| !allowed.contains(&key.as_str())) {
        errors.push(FieldError::new(format!("schedule.{key}"), "unknown", "is not allowed here"));
    }
    if kind == "once" {
        let starts = moment(object, "starts_at", &mut errors);
        let ends = moment(object, "ends_at", &mut errors);
        if let (Some(starts), Some(ends)) = (starts, ends)
            && ends <= starts
        {
            errors.push(FieldError::new("schedule.ends_at", "range", "must be after `starts_at`"));
        }
        return match (starts, ends) {
            (Some(starts_at), Some(ends_at)) if errors.is_empty() => {
                Ok(Schedule::Once { starts_at, ends_at })
            }
            _ => Err(errors),
        };
    }
    let start = clock(object, "start", &mut errors);
    let end = clock(object, "end", &mut errors);
    if let (Some(start), Some(end)) = (start, end)
        && start == end
    {
        errors.push(FieldError::new("schedule.end", "range", "must differ from `start`"));
    }
    let from = day(object, "from", &mut errors);
    let until = day(object, "until", &mut errors);
    if let (Some(from), Some(until)) = (from, until)
        && until < from
    {
        errors.push(FieldError::new("schedule.until", "range", "must not be before `from`"));
    }
    let days = if kind == "weekly" {
        let names: Vec<&str> = object
            .get("days")
            .and_then(Value::as_array)
            .map(|days| days.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let parsed: Vec<Weekday> = names
            .iter()
            .filter_map(|name| WEEKDAYS.iter().find(|(key, _)| key == name).map(|(_, day)| *day))
            .collect();
        let count = object.get("days").and_then(Value::as_array).map_or(0, Vec::len);
        if parsed.is_empty() || parsed.len() != count {
            errors.push(FieldError::new(
                "schedule.days",
                "invalid",
                "must list days among `mon`, `tue`, `wed`, `thu`, `fri`, `sat`, `sun`",
            ));
        }
        Some(parsed)
    } else {
        None
    };
    match (start, end) {
        (Some(start), Some(end)) if errors.is_empty() => {
            Ok(Schedule::Repeat { start, end, days, from, until })
        }
        _ => Err(errors),
    }
}

fn moment(
    object: &Map<String, Value>,
    key: &str,
    errors: &mut Vec<FieldError>,
) -> Option<Timestamp> {
    let found =
        object.get(key).and_then(Value::as_str).and_then(|raw| Timestamp::parse_rfc3339(raw).ok());
    if found.is_none() {
        errors.push(FieldError::new(
            format!("schedule.{key}"),
            "invalid",
            "must be a date and time (RFC 3339)",
        ));
    }
    found
}

fn clock(object: &Map<String, Value>, key: &str, errors: &mut Vec<FieldError>) -> Option<Time> {
    let found = object.get(key).and_then(Value::as_str).and_then(time::parse_clock);
    if found.is_none() {
        errors.push(FieldError::new(
            format!("schedule.{key}"),
            "invalid",
            "must be a local time written `HH:MM`",
        ));
    }
    found
}

fn day(object: &Map<String, Value>, key: &str, errors: &mut Vec<FieldError>) -> Option<Date> {
    let raw = object.get(key)?;
    let found = raw.as_str().and_then(|raw| Date::parse_from_str(raw, "%Y-%m-%d").ok());
    if found.is_none() {
        errors.push(FieldError::new(
            format!("schedule.{key}"),
            "invalid",
            "must be a date written `YYYY-MM-DD`",
        ));
    }
    found
}

impl Schedule {
    /// Lần chạy chứa `now` hay bắt đầu trong `[now - 1 ngày, now + HORIZON_DAYS ngày]`, theo thứ tự.
    pub(crate) fn windows(&self, now: Timestamp, zone: &TimeZone) -> Vec<Window> {
        match self {
            Schedule::Once { starts_at, ends_at } => {
                vec![Window { start: *starts_at, end: *ends_at }]
            }
            Schedule::Repeat { start, end, days, from, until } => {
                let today = now.date_in(zone);
                let mut found = Vec::new();
                for offset in -1..=HORIZON_DAYS {
                    let Some(date) = today.checked_add_days(offset) else { continue };
                    if from.is_some_and(|from| date < from)
                        || until.is_some_and(|until| date > until)
                    {
                        continue;
                    }
                    if days.as_ref().is_some_and(|days| !days.contains(&date.weekday())) {
                        continue;
                    }
                    let civil = date.to_jiff();
                    // Hết trước giờ bắt đầu: lần chạy qua nửa đêm, kết thúc ngày hôm sau.
                    let end_day = if end > start { Some(civil) } else { civil.tomorrow().ok() };
                    let (Some(window_start), Some(window_end)) = (
                        time::from_local(zone, civil.to_datetime(*start)),
                        end_day
                            .and_then(|end_day| time::from_local(zone, end_day.to_datetime(*end))),
                    ) else {
                        // Giờ không có ở địa phương (nhảy giờ mùa hè): bỏ lần đó.
                        continue;
                    };
                    found.push(Window { start: window_start, end: window_end });
                }
                found
            }
        }
    }

    /// Lần chạy đang chứa `now`, nếu có.
    pub(crate) fn current(&self, now: Timestamp, zone: &TimeZone) -> Option<Window> {
        self.windows(now, zone).into_iter().find(|window| window.start <= now && now < window.end)
    }

    /// Mốc bắt đầu hay kết thúc gần nhất sau `now`, trong [`HORIZON_DAYS`] ngày.
    pub(crate) fn next_boundary(&self, now: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
        let horizon = now.checked_add(SignedDuration::from_hours(24 * HORIZON_DAYS))?;
        self.windows(now, zone)
            .into_iter()
            .flat_map(|window| [window.start, window.end])
            .filter(|at| *at > now && *at <= horizon)
            .min()
    }

    /// Khoảng ngày lọc nhanh trong database: (`starts_at`, `ends_at`) của lịch `once`, `None` cho lịch lặp.
    pub(crate) fn bounds(&self) -> (Option<Timestamp>, Option<Timestamp>) {
        match self {
            Schedule::Once { starts_at, ends_at } => (Some(*starts_at), Some(*ends_at)),
            Schedule::Repeat { .. } => (None, None),
        }
    }

    /// Lịch ở dạng JSON đã chuẩn hoá (để lưu và trả về).
    pub(crate) fn to_json(&self) -> Value {
        match self {
            Schedule::Once { starts_at, ends_at } => {
                json!({"kind": "once", "starts_at": starts_at, "ends_at": ends_at})
            }
            Schedule::Repeat { start, end, days, from, until } => {
                let mut value = json!({
                    "kind": if days.is_some() { "weekly" } else { "daily" },
                    "start": start.strftime("%H:%M").to_string(),
                    "end": end.strftime("%H:%M").to_string(),
                });
                if let Some(days) = days {
                    value["days"] = json!(
                        days.iter()
                            .filter_map(|day| WEEKDAYS
                                .iter()
                                .find(|(_, known)| known == day)
                                .map(|(key, _)| *key))
                            .collect::<Vec<_>>()
                    );
                }
                if let Some(from) = from {
                    value["from"] = json!(from.to_string());
                }
                if let Some(until) = until {
                    value["until"] = json!(until.to_string());
                }
                value
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(raw: &str) -> Timestamp {
        Timestamp::parse_rfc3339(raw).unwrap()
    }

    fn saigon() -> TimeZone {
        time::zone("Asia/Ho_Chi_Minh").unwrap()
    }

    #[test]
    fn chu_de_tet_bat_dung_gio_tat_dung_gio() {
        let tet = check(&json!({"kind": "once", "starts_at": "2027-01-29T17:00:00Z", "ends_at": "2027-02-14T17:00:00Z"})).unwrap();
        assert!(tet.current(at("2027-01-29T16:59:59Z"), &saigon()).is_none());
        assert!(tet.current(at("2027-01-29T17:00:00Z"), &saigon()).is_some());
        assert!(
            tet.current(at("2027-02-14T17:00:00Z"), &saigon()).is_none(),
            "the end is excluded"
        );
        assert_eq!(
            tet.next_boundary(at("2027-01-29T10:00:00Z"), &saigon()),
            Some(at("2027-01-29T17:00:00Z"))
        );
    }

    #[test]
    fn gio_vang_qua_nua_dem() {
        // 22:00–02:00 giờ Việt Nam = 15:00–19:00 UTC.
        let late = check(&json!({"kind": "daily", "start": "22:00", "end": "02:00"})).unwrap();
        assert!(late.current(at("2027-03-01T15:30:00Z"), &saigon()).is_some());
        assert!(
            late.current(at("2027-03-01T18:59:00Z"), &saigon()).is_some(),
            "01:59 next local day"
        );
        assert!(late.current(at("2027-03-01T19:00:00Z"), &saigon()).is_none());
        assert_eq!(
            late.next_boundary(at("2027-03-01T12:00:00Z"), &saigon()),
            Some(at("2027-03-01T15:00:00Z"))
        );
    }

    #[test]
    fn lich_tuan_theo_mui_gio_cua_website() {
        let weekend = check(
            &json!({"kind": "weekly", "start": "00:00", "end": "23:59", "days": ["sat", "sun"]}),
        )
        .unwrap();
        // 2027-03-05 là thứ Sáu: 17:30 UTC đã là 00:30 thứ Bảy ở Việt Nam, còn ở UTC vẫn thứ Sáu.
        let friday_night = at("2027-03-05T17:30:00Z");
        assert!(weekend.current(friday_night, &saigon()).is_some());
        assert!(weekend.current(friday_night, &TimeZone::UTC).is_none());
        // Giờ mùa hè: 09:00 ở New York là 13:00 UTC mùa hè, 14:00 UTC mùa đông.
        let morning = check(&json!({"kind": "daily", "start": "09:00", "end": "10:00"})).unwrap();
        let york = time::zone("America/New_York").unwrap();
        assert!(morning.current(at("2027-07-01T13:30:00Z"), &york).is_some());
        assert!(morning.current(at("2027-01-04T14:30:00Z"), &york).is_some());
        assert!(morning.current(at("2027-01-04T13:30:00Z"), &york).is_none());
    }

    #[test]
    fn lich_sai_bao_moi_loi_mot_lan() {
        let errors = check(&json!({"kind": "weekly", "start": "25:00", "end": "x", "days": ["fun"], "from": "2027-13-01", "extra": 1})).unwrap_err();
        assert_eq!(errors.len(), 5, "{errors:?}");
        assert!(check(&json!({"kind": "once", "starts_at": "2027-01-02T00:00:00Z", "ends_at": "2027-01-01T00:00:00Z"})).is_err());
        assert!(check(&json!({"kind": "daily", "start": "10:00", "end": "10:00"})).is_err());
        assert!(check(&json!({"kind": "yearly"})).is_err());
    }

    #[test]
    fn json_chuan_hoa_doc_lai_duoc() {
        let raw = json!({"kind": "weekly", "start": "9:00", "end": "17:30", "days": ["mon", "fri"], "until": "2027-12-31"});
        let schedule = check(&raw).unwrap();
        let again = check(&schedule.to_json()).unwrap();
        assert_eq!(schedule, again);
        assert_eq!(schedule.to_json()["start"], "09:00");
    }
}
