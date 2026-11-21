//! Số liệu cho kích hoạt `metric` — cảnh báo bất thường (D-229): thống kê đơn giản, tất định, giải
//! thích được; không học máy.
//!
//! - `fact:<loại>`: số liệu báo cáo mà module khai cho event của mình (`Module::facts`, bước 38). Module
//!   này nghe các event đó và cộng theo giờ UTC vào `automations_metric_hours` — chỉ cho loại mà khách
//!   có luật `metric` dùng, nên khách không dùng cảnh báo không tốn một lần ghi nào. Giá trị của kỳ là
//!   tổng của NGÀY ĐỊA PHƯƠNG vừa hết (theo múi giờ của luật).
//! - `summary:<id>`: số liệu tóm tắt (bước 51 P4) đọc lúc kiểm, với quyền của người chạy luật; mỗi lần
//!   đọc ghi một mẫu, mức nền lấy từ các mẫu cũ. Tỉ lệ tính bằng phần vạn (1% = 100).
//!
//! Mức nền = trung bình của cùng thứ trong 4 tuần trước. Thiếu dữ liệu của một trong 4 ngày đó (mới
//! bắt đầu ghi, mẫu bị lỡ) thì KHÔNG báo — không báo sai còn hơn báo nhầm.

pub(crate) mod record;

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::OrgTarget;
use autoxide_kernel::error::Error;
use autoxide_kernel::report::{FactSource, Measure};
use autoxide_kernel::summary::{MetricValue, SummaryMetric, SummaryMetrics};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};

use crate::definition::{MetricOperator, MetricTrigger};
use crate::entity::metric_hour;
use crate::runner::Start;
use crate::service::Automations;
use crate::service::rules::Rule;

/// Số tuần của mức nền.
const BASELINE_WEEKS: i64 = 4;

/// Các số liệu dùng được làm kích hoạt.
#[derive(Clone, Default)]
pub(crate) struct MetricSources {
    /// Loại số liệu báo cáo → đơn vị.
    facts: BTreeMap<String, Measure>,
    summaries: SummaryMetrics,
}

impl MetricSources {
    pub(crate) fn new(sources: &[FactSource], summaries: SummaryMetrics) -> Self {
        let facts = sources
            .iter()
            .flat_map(|source| {
                source.kinds().iter().map(|kind| (kind.kind.to_owned(), kind.measure))
            })
            .collect();
        MetricSources { facts, summaries }
    }

    /// `name` (`fact:<loại>` / `summary:<id>`) là số liệu có thật.
    pub(crate) fn knows(&self, name: &str) -> bool {
        if let Some(kind) = name.strip_prefix("fact:") {
            return self.facts.contains_key(kind);
        }
        name.strip_prefix("summary:").is_some_and(|id| self.summaries.find(id).is_some())
    }

    /// Mọi số liệu: `(tên, mô tả, đơn vị)` cho danh mục kích hoạt.
    pub(crate) fn list(&self) -> Vec<(String, String, &'static str)> {
        let facts = self.facts.iter().map(|(kind, measure)| {
            (
                format!("fact:{kind}"),
                format!("Daily total of the report figure `{kind}`."),
                measure.as_str(),
            )
        });
        let summaries =
            self.summaries.infos().into_iter().map(|info| {
                (format!("summary:{}", info.id), info.definition.to_owned(), "reading")
            });
        facts.chain(summaries).collect()
    }

    fn summary(&self, id: &str) -> Option<Arc<dyn SummaryMetric>> {
        self.summaries.find(id)
    }
}

/// Một lần đọc số liệu và mức nền của nó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reading {
    pub day: Date,
    pub value: i64,
    /// `None`: chưa đủ dữ liệu nền.
    pub baseline: Option<i64>,
}

/// Số liệu có ra khỏi mức thường không.
pub(crate) fn alerts(trigger: &MetricTrigger, reading: &Reading) -> bool {
    let value = i128::from(reading.value);
    let limit = i128::from(trigger.value);
    match trigger.operator {
        MetricOperator::Below => value < limit,
        MetricOperator::Above => value > limit,
        MetricOperator::DropPct | MetricOperator::RisePct => {
            let Some(baseline) = reading.baseline.map(i128::from).filter(|baseline| *baseline > 0)
            else {
                return false;
            };
            let change = if trigger.operator == MetricOperator::DropPct {
                baseline - value
            } else {
                value - baseline
            };
            change * 100 >= limit * baseline
        }
    }
}

/// Câu giải thích (tiếng Anh, dữ liệu cho mẫu thông báo): giá trị, mức so sánh, phần trăm.
fn explanation(trigger: &MetricTrigger, reading: &Reading) -> String {
    let weekday = weekday_name(reading.day.weekday());
    match (trigger.operator.uses_baseline(), reading.baseline) {
        (true, Some(baseline)) if baseline > 0 => {
            let change =
                (i128::from(reading.value) - i128::from(baseline)) * 100 / i128::from(baseline);
            let direction = if change < 0 { "lower" } else { "higher" };
            format!(
                "{} on {} was {}, {}% {direction} than the average of the {BASELINE_WEEKS} previous {weekday}s: {baseline}.",
                trigger.metric,
                reading.day,
                reading.value,
                change.abs(),
            )
        }
        _ => format!(
            "{} on {} was {}, {} the threshold {}.",
            trigger.metric,
            reading.day,
            reading.value,
            if trigger.operator == MetricOperator::Below { "below" } else { "above" },
            trigger.value
        ),
    }
}

fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Monday => "Monday",
        Weekday::Tuesday => "Tuesday",
        Weekday::Wednesday => "Wednesday",
        Weekday::Thursday => "Thursday",
        Weekday::Friday => "Friday",
        Weekday::Saturday => "Saturday",
        Weekday::Sunday => "Sunday",
    }
}

/// Khoảng UTC của ngày địa phương `day` theo `zone`.
// Nửa đêm địa phương không có (nhảy giờ đúng lúc 0 giờ) thì lấy nửa đêm UTC, như trước.
fn day_range(day: Date, zone: &TimeZone) -> (Timestamp, Timestamp) {
    let start = |day: Date| {
        let midnight = day.to_jiff().to_datetime(jiff::civil::Time::midnight());
        autoxide_kernel::time::from_local(zone, midnight).unwrap_or_else(|| day.start_of_day_utc())
    };
    (start(day), start(days_before(day, -1)))
}

/// Ngày cách `day` về trước `days` ngày (âm: về sau). Ra ngoài lịch (năm ±9999) thì giữ `day`.
fn days_before(day: Date, days: i64) -> Date {
    day.checked_sub_days(days).unwrap_or(day)
}

/// Đầu giờ UTC chứa `at`.
pub(crate) fn hour_of(at: Timestamp) -> Timestamp {
    at.truncate(jiff::Unit::Hour)
}

impl Automations {
    /// Kiểm số liệu của luật `rule` ở mốc `slot`: đọc, so, và chạy luật khi số liệu ra khỏi mức
    /// thường — một lần mỗi kỳ (khoá `metric:<ngày>`).
    pub(crate) async fn check_metric(
        &self,
        tenant: &TenantContext,
        rule: &Rule,
        trigger: &MetricTrigger,
        slot: Timestamp,
        zone: &TimeZone,
    ) -> Result<(), Error> {
        let reading = if let Some(kind) = trigger.metric.strip_prefix("fact:") {
            self.fact_reading(tenant, kind, trigger.currency.as_deref(), slot, zone).await?
        } else if let Some(id) = trigger.metric.strip_prefix("summary:") {
            self.summary_reading(tenant, rule, id, trigger.currency.as_deref(), slot, zone).await?
        } else {
            None
        };
        let Some(reading) = reading else { return Ok(()) };
        if !alerts(trigger, &reading) {
            return Ok(());
        }
        let change_pct = reading.baseline.filter(|baseline| *baseline > 0).map(|baseline| {
            (i128::from(reading.value) - i128::from(baseline)) * 100 / i128::from(baseline)
        });
        let event = json!({
            "metric": trigger.metric,
            "day": reading.day,
            "value": reading.value,
            "baseline": reading.baseline,
            "change_pct": change_pct.map(|change| change as i64),
            "currency": trigger.currency,
            "operator": trigger.operator.as_str(),
            "threshold": trigger.value,
            "explanation": explanation(trigger, &reading),
        });
        let start = Start {
            trigger_key: format!("metric:{}", reading.day),
            event: None,
            context: json!({"event": event, "trigger": {"type": "metric", "checked_at": slot}}),
            depth: 1,
            triggered_by: None,
            skip: None,
        };
        if crate::runner::triggers::holds(rule, &start.context) {
            self.start_run(tenant, rule, start).await?;
        }
        Ok(())
    }

    /// Số liệu báo cáo: tổng của ngày địa phương vừa hết trước `slot`, và mức nền.
    async fn fact_reading(
        &self,
        tenant: &TenantContext,
        kind: &str,
        currency: Option<&str>,
        slot: Timestamp,
        zone: &TimeZone,
    ) -> Result<Option<Reading>, Error> {
        let source = format!("fact:{kind}");
        let day = days_before(slot.date_in(zone), 1);
        let earliest = days_before(day, BASELINE_WEEKS * 7);
        let (from, first_day_end) = day_range(earliest, zone);
        let (_, to) = day_range(day, zone);
        let rows = self.hours(tenant, &source, currency, from, to).await?;
        // Ghi bắt đầu từ lúc có luật dùng loại này: phải có số liệu từ ngày nền đầu tiên trở về trước,
        // không thì ngày "0" của mức nền có thể chỉ là chưa ghi.
        let first = metric_hour::Entity::find()
            .filter(metric_hour::Column::Source.eq(source.as_str()))
            .scoped(tenant)
            .order_by_asc(metric_hour::Column::Hour)
            .one(tenant.database().read())
            .await?;
        let tracked = first.is_some_and(|first| first.hour < first_day_end);
        let sum = |day: Date| {
            let (start, end) = day_range(day, zone);
            rows.iter()
                .filter(|row| row.hour >= start && row.hour < end)
                .map(|row| row.value)
                .sum::<i64>()
        };
        let baseline = tracked.then(|| {
            let total: i64 = (1..=BASELINE_WEEKS).map(|week| sum(days_before(day, 7 * week))).sum();
            total / BASELINE_WEEKS
        });
        Ok(Some(Reading { day, value: sum(day), baseline }))
    }

    /// Số liệu tóm tắt: đọc bây giờ với quyền của người chạy luật, ghi mẫu của hôm nay; mức nền là
    /// mẫu cuối của cùng thứ 4 tuần trước (thiếu một tuần thì không có).
    async fn summary_reading(
        &self,
        tenant: &TenantContext,
        rule: &Rule,
        id: &str,
        currency: Option<&str>,
        slot: Timestamp,
        zone: &TimeZone,
    ) -> Result<Option<Reading>, Error> {
        let Some(metric) = self.metrics.summary(id) else { return Ok(None) };
        let Some(principal) = self.rule_principal(tenant, rule).await? else {
            return Ok(None);
        };
        if !principal.has(&metric.permission()) {
            return Ok(None);
        }
        let target = OrgTarget::new(OrgId::from_uuid(rule.org_id), None);
        let (value, unit) = match metric.value(tenant, &principal, target).await? {
            MetricValue::Money { amount_minor, currency } => (amount_minor, currency),
            MetricValue::Count(count) => (i64::try_from(count).unwrap_or(i64::MAX), String::new()),
            MetricValue::Ratio { numerator, denominator } if denominator > 0 => {
                let ratio = u128::from(numerator) * 10_000 / u128::from(denominator);
                (i64::try_from(ratio).unwrap_or(i64::MAX), String::new())
            }
            MetricValue::Ratio { .. } => return Ok(None),
        };
        if currency.is_some_and(|currency| currency != unit) {
            return Ok(None);
        }
        let source = format!("summary:{id}");
        self.record_sample(tenant, &source, hour_of(slot), &unit, value).await?;
        let day = slot.date_in(zone);
        let (from, _) = day_range(days_before(day, BASELINE_WEEKS * 7), zone);
        let rows = self.hours(tenant, &source, Some(&unit), from, slot).await?;
        let mut samples = Vec::new();
        for week in 1..=BASELINE_WEEKS {
            let (start, end) = day_range(days_before(day, 7 * week), zone);
            // Mẫu cuối của ngày đó.
            let last = rows
                .iter()
                .filter(|row| row.hour >= start && row.hour < end)
                .max_by_key(|row| row.hour);
            samples.push(last.map(|row| row.value));
        }
        let baseline = samples
            .iter()
            .copied()
            .collect::<Option<Vec<i64>>>()
            .map(|values| values.iter().sum::<i64>() / BASELINE_WEEKS);
        Ok(Some(Reading { day, value, baseline }))
    }

    /// Các giờ của `source` trong `[from, to)`, lọc theo loại tiền khi có.
    async fn hours(
        &self,
        tenant: &TenantContext,
        source: &str,
        currency: Option<&str>,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<metric_hour::Model>, Error> {
        let mut select = metric_hour::Entity::find()
            .filter(metric_hour::Column::Source.eq(source))
            .filter(metric_hour::Column::Hour.gte(from))
            .filter(metric_hour::Column::Hour.lt(to))
            .scoped(tenant);
        if let Some(currency) = currency {
            select = select.filter(metric_hour::Column::Currency.eq(currency));
        }
        Ok(select.all(tenant.database().read()).await?)
    }
}

/// Dữ liệu `event` của một cảnh báo, cho mẫu luật và chạy thử khô.
pub(crate) fn sample_event(metric: &str) -> Value {
    json!({
        "metric": metric,
        "day": "2026-11-16",
        "value": 12_000_000,
        "baseline": 21_800_000,
        "change_pct": -44,
        "currency": "VND",
        "operator": "drop_pct",
        "threshold": 30,
        "explanation": format!("{metric} on 2026-11-16 was 12000000, 44% lower than the average of the 4 previous Mondays: 21800000."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(operator: MetricOperator, value: i64) -> MetricTrigger {
        MetricTrigger {
            metric: "fact:x.y".to_owned(),
            currency: None,
            operator,
            value,
            hour: 8,
            minute: 0,
        }
    }

    fn reading(value: i64, baseline: Option<i64>) -> Reading {
        Reading { day: Date::new(2026, 11, 16).unwrap_or_default(), value, baseline }
    }

    #[test]
    fn so_voi_muc_nen_va_nguong() {
        let drop = trigger(MetricOperator::DropPct, 30);
        assert!(alerts(&drop, &reading(12, Some(20))));
        assert!(!alerts(&drop, &reading(15, Some(20))));
        // Thiếu mức nền, hay mức nền bằng 0: không báo.
        assert!(!alerts(&drop, &reading(0, None)));
        assert!(!alerts(&drop, &reading(0, Some(0))));
        let rise = trigger(MetricOperator::RisePct, 50);
        assert!(alerts(&rise, &reading(30, Some(20))));
        assert!(!alerts(&trigger(MetricOperator::Below, 10), &reading(10, None)));
        assert!(alerts(&trigger(MetricOperator::Above, 10), &reading(11, None)));
        assert_eq!(
            explanation(&drop, &reading(12_000_000, Some(21_800_000))),
            "fact:x.y on 2026-11-16 was 12000000, 44% lower than the average of the 4 previous Mondays: 21800000."
        );
    }

    #[test]
    fn ngay_dia_phuong_thanh_khoang_utc() {
        let vietnam = &autoxide_kernel::time::zone("Asia/Ho_Chi_Minh").unwrap();
        let (start, end) = day_range(Date::new(2026, 11, 16).unwrap_or_default(), vietnam);
        assert_eq!(start.to_rfc3339(), "2026-11-15T17:00:00+00:00");
        assert_eq!(end.to_rfc3339(), "2026-11-16T17:00:00+00:00");
    }
}
