//! Hạn của yêu cầu của chủ thể dữ liệu (LG-4b, D-20261128-subject-request-fulfil): hạn trả lời và hạn thực hiện,
//! yêu cầu cũ chưa có hạn thực hiện (cả SQLite và Postgres).

use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use crate::tests::call;
use crate::tests_privacy::{app, at};

/// Quá hạn theo hai mốc (LG-4b): còn `received` mà qua hạn trả lời; đã nhận xử lý (`in_progress`) thì chỉ quá hạn
/// khi qua hạn thực hiện (phản đối: 15 ngày).
#[tokio::test]
async fn yeu_cau_qua_han_theo_han_tra_loi_va_han_thuc_hien() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let record = async |days: i64, subject: &str| {
            let received = Timestamp::now() - SignedDuration::from_hours(24 * days);
            let input = json!({"kind": "objection", "channel": "phone", "subject": subject,
                               "received_at": received.to_rfc3339()});
            let (status, recorded) =
                call(&app, "POST", "/subject-requests", "admin", Some(input)).await;
            assert_eq!(status, StatusCode::CREATED, "{recorded}");
            let fulfil = at(&recorded["data"]["fulfil_due_at"]) - received;
            assert_eq!(fulfil, SignedDuration::from_hours(24 * 15), "{recorded}");
            recorded["data"]["id"].as_str().unwrap().to_owned()
        };
        let answered = record(5, "0912345678").await;
        let late = record(20, "0912345679").await;
        let progress = json!({"status": "in_progress"});
        for id in [&answered, &late] {
            let uri = format!("/subject-requests/{id}");
            let (status, body) = call(&app, "PATCH", &uri, "admin", Some(progress.clone())).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        let unanswered = record(5, "0912345670").await;
        let (_, answered_row) =
            call(&app, "GET", &format!("/subject-requests/{answered}"), "admin", None).await;
        assert_eq!(answered_row["data"]["overdue"], false, "{answered_row}");
        let (_, overdue) = call(&app, "GET", "/subject-requests?overdue=true", "admin", None).await;
        let mut ids: Vec<&str> = overdue["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        ids.sort_unstable();
        let mut expected = vec![late.as_str(), unanswered.as_str()];
        expected.sort_unstable();
        assert_eq!(ids, expected, "{overdue}");
        let (_, overview) = call(&app, "GET", "/overview", "admin", None).await;
        assert_eq!(overview["data"]["subject_requests"], json!({"open": 3, "overdue": 2}));
        // Yêu cầu ghi trước khi có hạn thực hiện (`fulfil_due_at` trống): đang xử lý mà qua hạn trả lời vẫn là quá
        // hạn, như trước m0025 (kiểm độc lập #620).
        let id: Uuid = answered.parse().unwrap();
        crate::entity::subject_request::Entity::update_many()
            .col_expr(
                crate::entity::subject_request::Column::FulfilDueAt,
                Expr::value(Option::<Timestamp>::None),
            )
            .filter(crate::entity::subject_request::Column::Id.eq(id))
            .exec(app.database().write())
            .await
            .unwrap();
        let (_, old_row) =
            call(&app, "GET", &format!("/subject-requests/{answered}"), "admin", None).await;
        assert_eq!(old_row["data"]["overdue"], true, "{old_row}");
        let (_, overdue) = call(&app, "GET", "/subject-requests?overdue=true", "admin", None).await;
        assert_eq!(overdue["data"].as_array().unwrap().len(), 3, "{overdue}");
    })
    .await
    .unwrap();
}
