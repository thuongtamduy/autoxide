//! Hạn trả lời (SLA): tính theo giờ làm và ngày nghỉ của nút, dừng khi chờ người mua, job đánh
//! dấu quá hạn và tự đóng, hộp việc, báo cáo.

use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{call, harness, service_of};
use crate::entity::ticket;

/// Chính sách: làm cả tuần 00:00–23:59 giờ UTC, không ngày nghỉ — để tính được bằng tay.
fn always_open() -> Value {
    let hours: Vec<Value> =
        (1..=7).map(|day| json!({"day": day, "opens": "00:00", "closes": "23:59"})).collect();
    json!({
        "categories": ["order", "other"],
        "first_response_minutes": {"low": 600, "normal": 60, "high": 30, "urgent": 10},
        "resolve_minutes": {"low": 6000, "normal": 600, "high": 300, "urgent": 100},
        "business_hours": hours, "holidays": [], "utc_offset_minutes": 0, "auto_close_days": 2,
        "guest_tickets": false, "inbound_email": false, "auto_reply": false,
        "auto_reply_min_confidence_bp": 9000, "auto_reply_max_per_thread": 2, "auto_reply_blocked_terms": []
    })
}

async fn set_policy(app: &TestApp, policy: Value) {
    let root = app.default_tenant().id().as_uuid();
    let (status, body) =
        call(app, "PUT", &format!("/policies/{root}"), "agent", Some(policy)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

fn time(value: &Value) -> Timestamp {
    value.as_str().unwrap().parse().unwrap()
}

async fn shift(app: &TestApp, id: &str, column: ticket::Column, at: Timestamp) {
    ticket::Entity::update_many()
        .col_expr(column, Expr::value(at))
        .filter(ticket::Column::Id.eq(id.parse::<Uuid>().unwrap()))
        .exec(app.database().write())
        .await
        .unwrap();
}

#[tokio::test]
async fn dong_ho_sla_dung_khi_cho_nguoi_mua() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        set_policy(app, always_open()).await;
        let body = json!({"category": "order", "subject": "Hỏi", "body": "?"});
        let (_, opened) = call(app, "POST", "/public/tickets", "buyer", Some(body)).await;
        let data = &opened["data"];
        let id = data["id"].as_str().unwrap().to_owned();
        let created = time(&data["created_at"]);
        // Lịch nghỉ một phút mỗi đêm (23:59–24:00): hạn có thể lùi tới một phút.
        let near = |due: Timestamp, minutes: i64| {
            let gap = due - created - SignedDuration::from_mins(minutes);
            (SignedDuration::ZERO..=SignedDuration::from_secs(61)).contains(&gap)
        };
        assert!(near(time(&data["first_response_due_at"]), 60), "{data}");
        let resolve = time(&data["resolve_due_at"]);
        assert!(near(resolve, 600), "{data}");

        // Loại `product` không còn trong danh sách của chính sách.
        let other = json!({"category": "product", "subject": "Hỏi", "body": "?"});
        let (status, _) = call(app, "POST", "/public/tickets", "buyer", Some(other)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        // Nhân viên trả lời: chờ người mua, đồng hồ dừng.
        call(app, "POST", &format!("/tickets/{id}/messages"), "agent", Some(json!({"body": "Đã nhận"})))
            .await;
        // Người mua trả lời sau 2 giờ: hạn giải quyết lùi đúng 2 giờ.
        let paused_at = Timestamp::now() - SignedDuration::from_hours(2);
        shift(app, &id, ticket::Column::SlaPausedAt, paused_at).await;
        call(app, "POST", &format!("/me/tickets/{id}/messages"), "buyer", Some(json!({"body": "Còn nữa"})))
            .await;
        let (_, got) = call(app, "GET", &format!("/tickets/{id}"), "agent", None).await;
        let moved = time(&got["data"]["resolve_due_at"]) - resolve;
        assert!((SignedDuration::from_mins(119)..=SignedDuration::from_mins(122)).contains(&moved), "{moved}");

        // Khách vãng lai bị tắt bởi chính sách.
        let guest = json!({"category": "order", "subject": "x", "body": "y", "email": "g@example.com",
                           "form_token": super::form_token(app).await});
        let (status, problem) = call(app, "POST", "/public/tickets", "", Some(guest)).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::FORBIDDEN, Some("support.guest_tickets_disabled")));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn job_danh_dau_qua_han_va_tu_dong_phieu() {
    TestDatabase::run(async |test| {
        let harness = harness(test).await;
        let app = &harness.app;
        set_policy(app, always_open()).await;
        let body = json!({"category": "order", "subject": "Chậm", "body": "?"});
        let (_, late) = call(app, "POST", "/public/tickets", "buyer", Some(body.clone())).await;
        let late = late["data"]["id"].as_str().unwrap().to_owned();
        let (_, solved) = call(app, "POST", "/public/tickets", "buyer", Some(body)).await;
        let solved = solved["data"]["id"].as_str().unwrap().to_owned();
        shift(
            app,
            &late,
            ticket::Column::FirstResponseDueAt,
            Timestamp::now() - SignedDuration::from_mins(1),
        )
        .await;
        call(
            app,
            "PATCH",
            &format!("/tickets/{solved}"),
            "agent",
            Some(json!({"status": "solved"})),
        )
        .await;
        shift(
            app,
            &solved,
            ticket::Column::SolvedAt,
            Timestamp::now() - SignedDuration::from_hours(24 * 3),
        )
        .await;

        let service = service_of(app);
        let tenant = app.default_tenant();
        assert_eq!(service.mark_breached(&tenant, Timestamp::now()).await.unwrap(), 1);
        assert_eq!(
            service.mark_breached(&tenant, Timestamp::now()).await.unwrap(),
            0,
            "once per ticket"
        );
        assert_eq!(service.auto_close(&tenant, Timestamp::now()).await.unwrap(), 1);

        let (_, breached) = call(app, "GET", "/tickets?breached=true", "agent", None).await;
        assert_eq!(breached["data"].as_array().unwrap().len(), 1);
        let (_, got) = call(app, "GET", &format!("/tickets/{solved}"), "agent", None).await;
        assert_eq!(got["data"]["status"], "closed");

        // Hộp việc: phiếu chờ nhân viên, quá hạn thì gấp.
        let work = app.get("/api/v1/kernel/work").token("agent").send().await.unwrap();
        assert_eq!(work.status(), StatusCode::OK, "{}", work.text());
        let items = work.json()["data"]["items"].clone();
        let mine: Vec<&Value> = items
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["source"] == "support.tickets")
            .collect();
        assert_eq!(mine.len(), 1, "{items}");

        // Báo cáo.
        let from = (Timestamp::now() - SignedDuration::from_hours(24)).to_rfc3339();
        let to = (Timestamp::now() + SignedDuration::from_hours(24)).to_rfc3339();
        let uri = format!("/reports/summary?from={}&to={}", urlencode(&from), urlencode(&to));
        let (status, report) = call(app, "GET", &uri, "agent", None).await;
        assert_eq!(status, StatusCode::OK, "{report}");
        assert_eq!(report["data"]["tickets"], 2);
        assert_eq!(report["data"]["breached"], 1);
        assert_eq!(report["data"]["breach_rate_bp"], 5000);
    })
    .await
    .unwrap();
}

fn urlencode(raw: &str) -> String {
    raw.replace('+', "%2B").replace(':', "%3A")
}
