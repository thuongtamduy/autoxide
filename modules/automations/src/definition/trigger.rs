//! Kích hoạt của luật: một event đã khai, lịch đơn giản (mỗi giờ / ngày / tuần theo múi giờ), gọi
//! tay, hay số liệu ra khỏi mức thường (D-229).
//!
//! ```json
//! {"type": "event", "event": "orders.order_placed"}
//! {"type": "schedule", "every": "week", "weekday": "monday", "hour": 9, "minute": 0}
//! {"type": "manual"}
//! {"type": "metric", "metric": "fact:orders.revenue", "operator": "drop_pct", "value": 30, "hour": 8}
//! ```
//!
//! Lịch không dùng cú pháp cron (D-20261118-step58-automations): ba nhịp cố định đủ cho chủ cửa hàng,
//! kiểm được và giải thích được bằng một câu.

use autoxide_kernel::error::FieldError;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use serde_json::Value;

/// Nhịp của lịch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Every {
    Hour,
    Day,
    Week,
}

/// Lịch lặp lại: phút (mỗi giờ), giờ:phút (mỗi ngày), thứ + giờ:phút (mỗi tuần), theo giờ địa phương.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Recurrence {
    pub every: Every,
    pub hour: u32,
    pub minute: u32,
    pub weekday: Weekday,
}

/// So số liệu thế nào.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetricOperator {
    /// Thấp hơn mức nền (trung bình cùng thứ 4 tuần trước) ít nhất `value` phần trăm.
    DropPct,
    /// Cao hơn mức nền ít nhất `value` phần trăm.
    RisePct,
    /// Dưới ngưỡng cố định `value`.
    Below,
    /// Trên ngưỡng cố định `value`.
    Above,
}

impl MetricOperator {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            MetricOperator::DropPct => "drop_pct",
            MetricOperator::RisePct => "rise_pct",
            MetricOperator::Below => "below",
            MetricOperator::Above => "above",
        }
    }

    /// So với mức nền (không phải ngưỡng cố định).
    pub(crate) fn uses_baseline(self) -> bool {
        matches!(self, MetricOperator::DropPct | MetricOperator::RisePct)
    }
}

/// Kích hoạt theo số liệu: kiểm mỗi ngày lúc `hour:minute` giờ địa phương.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetricTrigger {
    /// `fact:<loại số liệu báo cáo>` hay `summary:<id số liệu tóm tắt>`.
    pub metric: String,
    /// Chỉ cộng số tiền của loại tiền này (số liệu tiền); `None`: mọi dòng.
    pub currency: Option<String>,
    pub operator: MetricOperator,
    pub value: i64,
    pub hour: u32,
    pub minute: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Trigger {
    Event { event: String },
    Schedule(Recurrence),
    Manual,
    Metric(MetricTrigger),
}

impl Trigger {
    /// Tên loại: `event`, `schedule`, `manual`, `metric` (cột `trigger_type`).
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Trigger::Event { .. } => "event",
            Trigger::Schedule(_) => "schedule",
            Trigger::Manual => "manual",
            Trigger::Metric(_) => "metric",
        }
    }

    /// Cột `trigger_ref`: tên event hay số liệu.
    pub(crate) fn reference(&self) -> Option<&str> {
        match self {
            Trigger::Event { event } => Some(event),
            Trigger::Metric(metric) => Some(&metric.metric),
            Trigger::Schedule(_) | Trigger::Manual => None,
        }
    }

    /// Lần chạy theo lịch kế tiếp sau `after` (luật theo lịch hay số liệu); `None` với luật khác.
    pub(crate) fn next_run(&self, after: DateTime<Utc>, zone: Tz) -> Option<DateTime<Utc>> {
        match self {
            Trigger::Schedule(recurrence) => Some(recurrence.next_after(after, zone)),
            Trigger::Metric(metric) => Some(
                Recurrence {
                    every: Every::Day,
                    hour: metric.hour,
                    minute: metric.minute,
                    weekday: Weekday::Mon,
                }
                .next_after(after, zone),
            ),
            Trigger::Event { .. } | Trigger::Manual => None,
        }
    }

    /// Đọc kích hoạt ở `field`; lỗi ghi vào `errors`. Trả thêm múi giờ khai trong kích hoạt, nếu có.
    pub(crate) fn parse(
        value: &Value,
        field: &str,
        errors: &mut Vec<FieldError>,
    ) -> Option<(Trigger, Option<Tz>)> {
        let Some(object) = value.as_object() else {
            errors.push(FieldError::new(field, "invalid", "must be an object with `type`"));
            return None;
        };
        let before = errors.len();
        let zone = match object.get("timezone") {
            None | Some(Value::Null) => None,
            Some(raw) => match raw.as_str().and_then(|text| text.parse::<Tz>().ok()) {
                Some(zone) => Some(zone),
                None => {
                    errors.push(FieldError::new(
                        format!("{field}.timezone"),
                        "invalid",
                        "must be an IANA time zone such as `Asia/Ho_Chi_Minh`",
                    ));
                    None
                }
            },
        };
        let allowed: &[&str] = match object.get("type").and_then(Value::as_str) {
            Some("event") => &["type", "event"],
            Some("schedule") => &["type", "every", "weekday", "hour", "minute", "timezone"],
            Some("manual") => &["type"],
            Some("metric") => {
                &["type", "metric", "currency", "operator", "value", "hour", "minute", "timezone"]
            }
            _ => {
                errors.push(FieldError::new(
                    format!("{field}.type"),
                    "invalid",
                    "must be one of event, schedule, manual, metric",
                ));
                return None;
            }
        };
        for key in object.keys().filter(|key| !allowed.contains(&key.as_str())) {
            errors.push(FieldError::new(
                format!("{field}.{key}"),
                "unknown",
                "is not a known field",
            ));
        }
        let trigger = match object.get("type").and_then(Value::as_str) {
            Some("event") => {
                let event = object.get("event").and_then(Value::as_str).unwrap_or_default();
                if event.is_empty() {
                    errors.push(FieldError::new(
                        format!("{field}.event"),
                        "required",
                        "is required",
                    ));
                }
                Trigger::Event { event: event.to_owned() }
            }
            Some("schedule") => {
                let every = match object.get("every").and_then(Value::as_str) {
                    Some("hour") => Every::Hour,
                    Some("day") => Every::Day,
                    Some("week") => Every::Week,
                    _ => {
                        errors.push(FieldError::new(
                            format!("{field}.every"),
                            "invalid",
                            "must be one of hour, day, week",
                        ));
                        Every::Day
                    }
                };
                let weekday = match object.get("weekday") {
                    None | Some(Value::Null) if every != Every::Week => Weekday::Mon,
                    raw => match raw.and_then(Value::as_str).and_then(weekday) {
                        Some(day) => day,
                        None => {
                            errors.push(FieldError::new(
                                format!("{field}.weekday"),
                                "invalid",
                                "must be a day name such as `monday` (required when `every` is `week`)",
                            ));
                            Weekday::Mon
                        }
                    },
                };
                let hour = number(object, "hour", 23, field, errors);
                let minute = number(object, "minute", 59, field, errors);
                Trigger::Schedule(Recurrence { every, hour, minute, weekday })
            }
            Some("manual") => Trigger::Manual,
            _ => {
                let metric = object.get("metric").and_then(Value::as_str).unwrap_or_default();
                let named = metric.split_once(':').is_some_and(|(kind, id)| {
                    ["fact", "summary"].contains(&kind) && !id.is_empty()
                });
                if !named {
                    errors.push(FieldError::new(
                        format!("{field}.metric"),
                        "invalid",
                        "must be `fact:<kind>` or `summary:<metric id>` from the catalog",
                    ));
                }
                let currency = match object.get("currency") {
                    None | Some(Value::Null) => None,
                    Some(raw) => match raw.as_str().filter(|code| {
                        code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase())
                    }) {
                        Some(code) => Some(code.to_owned()),
                        None => {
                            errors.push(FieldError::new(
                                format!("{field}.currency"),
                                "invalid",
                                "must be an ISO 4217 code such as `VND`",
                            ));
                            None
                        }
                    },
                };
                let operator = match object.get("operator").and_then(Value::as_str) {
                    Some("drop_pct") => MetricOperator::DropPct,
                    Some("rise_pct") => MetricOperator::RisePct,
                    Some("below") => MetricOperator::Below,
                    Some("above") => MetricOperator::Above,
                    _ => {
                        errors.push(FieldError::new(
                            format!("{field}.operator"),
                            "invalid",
                            "must be one of drop_pct, rise_pct, below, above",
                        ));
                        MetricOperator::Above
                    }
                };
                let value = object.get("value").and_then(Value::as_i64);
                let value = match value {
                    Some(value) if operator.uses_baseline() && (1..=1000).contains(&value) => value,
                    Some(value) if !operator.uses_baseline() && value >= 0 => value,
                    _ => {
                        errors.push(FieldError::new(
                            format!("{field}.value"),
                            "invalid",
                            "must be a whole number: a percentage from 1 to 1000 for drop_pct and \
                             rise_pct, a threshold of 0 or more for below and above",
                        ));
                        0
                    }
                };
                let hour = number(object, "hour", 23, field, errors);
                let minute = number(object, "minute", 59, field, errors);
                Trigger::Metric(MetricTrigger {
                    metric: metric.to_owned(),
                    currency,
                    operator,
                    value,
                    hour,
                    minute,
                })
            }
        };
        (errors.len() == before).then_some((trigger, zone))
    }
}

/// Trường số nguyên `key` từ 0 tới `max` (mặc định 0).
fn number(
    object: &serde_json::Map<String, Value>,
    key: &str,
    max: u32,
    field: &str,
    errors: &mut Vec<FieldError>,
) -> u32 {
    match object.get(key) {
        None | Some(Value::Null) => 0,
        Some(raw) => match raw.as_u64().and_then(|value| u32::try_from(value).ok()) {
            Some(value) if value <= max => value,
            _ => {
                errors.push(
                    FieldError::new(format!("{field}.{key}"), "range", "must be from 0 to {max}")
                        .with_param("max", max),
                );
                0
            }
        },
    }
}

fn weekday(name: &str) -> Option<Weekday> {
    Some(match name {
        "monday" => Weekday::Mon,
        "tuesday" => Weekday::Tue,
        "wednesday" => Weekday::Wed,
        "thursday" => Weekday::Thu,
        "friday" => Weekday::Fri,
        "saturday" => Weekday::Sat,
        "sunday" => Weekday::Sun,
        _ => return None,
    })
}

impl Recurrence {
    /// Mốc đầu tiên SAU `after` theo giờ địa phương của `zone`. Giờ không có ở địa phương (chuyển giờ
    /// mùa hè) thì bỏ qua mốc đó.
    pub(crate) fn next_after(&self, after: DateTime<Utc>, zone: Tz) -> DateTime<Utc> {
        let local = after.with_timezone(&zone);
        let today = local.date_naive();
        match self.every {
            Every::Hour => {
                // Mốc ở giờ hiện tại hoặc các giờ sau (tối đa hai ngày để qua mọi chỗ chuyển giờ).
                let start = local.naive_local().date().and_hms_opt(local.hour_of_day(), 0, 0);
                let Some(start) = start else { return after + Duration::hours(1) };
                (0..48)
                    .filter_map(|offset| {
                        let naive = start
                            + Duration::hours(offset)
                            + Duration::minutes(i64::from(self.minute));
                        zone.from_local_datetime(&naive).earliest()
                    })
                    .map(|at| at.with_timezone(&Utc))
                    .find(|at| *at > after)
                    .unwrap_or(after + Duration::hours(1))
            }
            Every::Day | Every::Week => {
                let time =
                    NaiveTime::from_hms_opt(self.hour, self.minute, 0).unwrap_or(NaiveTime::MIN);
                (0..16)
                    .filter_map(|offset| today.checked_add_signed(Duration::days(offset)))
                    .filter(|day: &NaiveDate| {
                        self.every == Every::Day || day.weekday() == self.weekday
                    })
                    .filter_map(|day| zone.from_local_datetime(&day.and_time(time)).earliest())
                    .map(|at| at.with_timezone(&Utc))
                    .find(|at| *at > after)
                    .unwrap_or(after + Duration::days(1))
            }
        }
    }
}

/// Giờ trong ngày của một thời điểm địa phương (chrono không có hàm ngắn cho việc này).
trait HourOfDay {
    fn hour_of_day(&self) -> u32;
}

impl<Z: TimeZone> HourOfDay for DateTime<Z> {
    fn hour_of_day(&self) -> u32 {
        use chrono::Timelike;
        self.naive_local().hour()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().to_utc()
    }

    fn parse(value: Value) -> Result<Trigger, Vec<FieldError>> {
        let mut errors = Vec::new();
        Trigger::parse(&value, "trigger", &mut errors).map(|(trigger, _)| trigger).ok_or(errors)
    }

    #[test]
    fn lich_theo_gio_dia_phuong() {
        let vietnam = chrono_tz::Asia::Ho_Chi_Minh;
        // 2026-11-16 là thứ Hai. 08:00 giờ Việt Nam = 01:00 UTC.
        let weekly = Recurrence { every: Every::Week, hour: 8, minute: 0, weekday: Weekday::Mon };
        assert_eq!(
            weekly.next_after(at("2026-11-16T00:30:00Z"), vietnam),
            at("2026-11-16T01:00:00Z")
        );
        assert_eq!(
            weekly.next_after(at("2026-11-16T01:00:00Z"), vietnam),
            at("2026-11-23T01:00:00Z")
        );
        let daily = Recurrence { every: Every::Day, ..weekly };
        assert_eq!(
            daily.next_after(at("2026-11-16T01:00:00Z"), vietnam),
            at("2026-11-17T01:00:00Z")
        );
        let hourly = Recurrence { every: Every::Hour, minute: 15, ..weekly };
        assert_eq!(
            hourly.next_after(at("2026-11-16T01:20:00Z"), vietnam),
            at("2026-11-16T02:15:00Z")
        );
        assert_eq!(
            hourly.next_after(at("2026-11-16T01:10:00Z"), vietnam),
            at("2026-11-16T01:15:00Z")
        );
    }

    #[test]
    fn doc_kich_hoat_va_bao_loi() {
        assert_eq!(parse(json!({"type": "manual"})).unwrap(), Trigger::Manual);
        assert_eq!(
            parse(json!({"type": "schedule", "every": "week", "weekday": "friday", "hour": 17}))
                .unwrap(),
            Trigger::Schedule(Recurrence {
                every: Every::Week,
                hour: 17,
                minute: 0,
                weekday: Weekday::Fri
            })
        );
        let errors =
            parse(json!({"type": "schedule", "every": "week", "hour": 30, "cron": "* * *"}))
                .unwrap_err();
        let fields: Vec<&str> = errors.iter().map(|error| error.field.as_str()).collect();
        assert_eq!(fields, vec!["trigger.cron", "trigger.weekday", "trigger.hour"]);
        let errors = parse(
            json!({"type": "metric", "metric": "orders", "operator": "drop_pct", "value": 0}),
        )
        .unwrap_err();
        assert_eq!(errors.len(), 2);
        let metric = parse(
            json!({"type": "metric", "metric": "fact:x.y", "operator": "below", "value": 10}),
        );
        assert!(matches!(metric, Ok(Trigger::Metric(MetricTrigger { value: 10, .. }))));
    }
}
