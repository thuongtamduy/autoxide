//! Cảnh báo bất thường (D-229): số liệu so với mức nền 4 tuần hay ngưỡng, một lần mỗi kỳ, thiếu dữ
//! liệu nền thì không báo.

use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use jiff::SignedDuration;
use jiff::tz::TimeZone;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::{Value, json};

use super::Harness;
use crate::definition::{MetricTrigger, Trigger};
use crate::entity::metric_hour;
use crate::service::rules::{Rule, parse_json};

/// 2026-11-16 là thứ Hai; kiểm lúc 08:00 giờ Việt Nam (01:00 UTC) cho ngày Chủ nhật 15/11.
fn slot() -> Timestamp {
    Timestamp::parse_rfc3339("2026-11-16T01:00:00Z").unwrap()
}

/// Doanh thu `value` vào giữa ngày địa phương `days_before` ngày trước ngày được kiểm (15/11).
async fn revenue(tenant: &TenantContext, days_before: i64, value: i64) {
    // 12:00 giờ Việt Nam = 05:00 UTC.
    let hour = Timestamp::parse_rfc3339("2026-11-15T05:00:00Z").unwrap()
        - SignedDuration::from_hours(24 * days_before);
    metric_hour::ActiveModel {
        tenant_id: Set(tenant.id().as_uuid()),
        source: Set("fact:shop.revenue".to_owned()),
        hour: Set(hour),
        currency: Set("VND".to_owned()),
        value: Set(value),
        samples: Set(1),
    }
    .insert(tenant.database().write())
    .await
    .unwrap();
}

async fn drop_rule(harness: &Harness) -> Rule {
    let trigger = json!({
        "type": "metric", "metric": "fact:shop.revenue", "currency": "VND", "operator": "drop_pct",
        "value": 30, "hour": 8, "timezone": "Asia/Ho_Chi_Minh"
    });
    harness
        .rule(
            trigger,
            json!(null),
            json!([{"notify": {"title": "Revenue dropped", "body": "{{ event.explanation }}"}}]),
        )
        .await
}

fn metric_of(rule: &Rule) -> MetricTrigger {
    let mut errors = Vec::new();
    match Trigger::parse(&parse_json(&rule.trigger), "trigger", &mut errors) {
        Some((Trigger::Metric(metric), _)) => metric,
        _ => panic!("not a metric rule"),
    }
}

async fn check(harness: &Harness, rule: &Rule) {
    let zone: TimeZone = autoxide_kernel::time::zone(&rule.timezone).unwrap();
    harness
        .service
        .check_metric(&harness.tenant, rule, &metric_of(rule), slot(), &zone)
        .await
        .unwrap();
}

#[tokio::test]
async fn so_lieu_tut_so_voi_muc_nen_thi_bao() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let rule = drop_rule(&harness).await;
        // Bốn Chủ nhật trước: 1 000 000 mỗi ngày; hôm qua (Chủ nhật): 500 000. Ngày khác không tính.
        for week in 1..=4 {
            revenue(&harness.tenant, 7 * week, 1_000_000).await;
        }
        revenue(&harness.tenant, 3, 9_000_000).await;
        revenue(&harness.tenant, 0, 500_000).await;
        check(&harness, &rule).await;
        let runs = harness.runs(rule.id).await;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].trigger_key, "metric:2026-11-15");
        let context: Value = parse_json(&runs[0].context);
        assert_eq!(context["event"]["value"], json!(500_000));
        assert_eq!(context["event"]["baseline"], json!(1_000_000));
        assert_eq!(context["event"]["change_pct"], json!(-50));
        assert_eq!(
            context["event"]["explanation"],
            json!("fact:shop.revenue on 2026-11-15 was 500000, 50% lower than the average of the 4 previous Sundays: 1000000.")
        );
        // Chỉ giảm 20% (chưa tới 30%): không báo.
        let calm = drop_rule(&harness).await;
        metric_hour::Entity::update_many()
            .col_expr(metric_hour::Column::Value, sea_orm::sea_query::Expr::value(800_000i64))
            .filter(metric_hour::Column::Value.eq(500_000i64))
            .scoped(&harness.tenant)
            .exec(harness.tenant.database().write())
            .await
            .unwrap();
        check(&harness, &calm).await;
        assert!(harness.runs(calm.id).await.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mot_ky_chi_bao_mot_lan() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let rule = drop_rule(&harness).await;
        for week in 1..=4 {
            revenue(&harness.tenant, 7 * week, 1_000_000).await;
        }
        check(&harness, &rule).await;
        check(&harness, &rule).await;
        assert_eq!(harness.runs(rule.id).await.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_lieu_thieu_du_lieu_nen_khong_bao_sai() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let rule = drop_rule(&harness).await;
        // Mới ghi được hai tuần: chưa có mức nền 4 tuần, doanh thu bằng 0 cũng không báo.
        for week in 1..=2 {
            revenue(&harness.tenant, 7 * week, 1_000_000).await;
        }
        check(&harness, &rule).await;
        assert!(harness.runs(rule.id).await.is_empty());
        // Ngưỡng cố định không cần mức nền.
        let trigger = json!({"type": "metric", "metric": "fact:shop.revenue", "operator": "below", "value": 1, "hour": 8});
        let below = harness.rule(trigger, json!(null), json!([{"wait": {"minutes": 1}}])).await;
        check(&harness, &below).await;
        assert_eq!(harness.runs(below.id).await.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_lieu_bao_cao_duoc_cong_tu_event() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        // Chưa có luật dùng `fact:shop.revenue`: không ghi gì.
        harness.shop.place(&harness.tenant, 100).await;
        harness.settle().await;
        let total = async || -> i64 {
            metric_hour::Entity::find()
                .filter(metric_hour::Column::Source.eq("fact:shop.revenue"))
                .scoped(&harness.tenant)
                .all(harness.tenant.database().read())
                .await
                .unwrap()
                .iter()
                .map(|row| row.value)
                .sum()
        };
        assert_eq!(total().await, 0);
        drop_rule(&harness).await;
        harness.shop.place(&harness.tenant, 100).await;
        harness.shop.place(&harness.tenant, 250).await;
        harness.settle().await;
        assert_eq!(total().await, 350);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_lieu_tom_tat_doc_bang_quyen_nguoi_chay() {
    TestDatabase::run(async |test| {
        let harness = Harness::new(test).await;
        let trigger = json!({"type": "metric", "metric": "summary:shop.pending", "operator": "above", "value": 10, "hour": 8});
        let rule = harness.rule(trigger, json!(null), json!([{"wait": {"minutes": 1}}])).await;
        harness.shop.counters.pending.store(5, std::sync::atomic::Ordering::SeqCst);
        check(&harness, &rule).await;
        assert!(harness.runs(rule.id).await.is_empty());
        harness.shop.counters.pending.store(12, std::sync::atomic::Ordering::SeqCst);
        let zone: TimeZone = autoxide_kernel::time::zone(&rule.timezone).unwrap();
        let next_day = slot() + SignedDuration::from_hours(24);
        harness.service.check_metric(&harness.tenant, &rule, &metric_of(&rule), next_day, &zone).await.unwrap();
        let runs = harness.runs(rule.id).await;
        assert_eq!(runs.len(), 1);
        assert_eq!(parse_json(&runs[0].context)["event"]["value"], json!(12));
    })
    .await
    .unwrap();
}
