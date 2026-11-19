//! Hỗ trợ khách hàng trên chương trình thật (cả SQLite và Postgres): người mua mở phiếu, nhân viên
//! trả lời rồi chuyển thành khiếu nại người tiêu dùng — `compliance` thật nhận vụ việc có đích là
//! phiếu (qua sổ đối tượng), gọi lại không tạo vụ thứ hai.

use autoxide_kernel::access::{Principal, Scope};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/support{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

#[tokio::test]
async fn phieu_thanh_khieu_nai_cua_compliance_mot_lan() {
    TestDatabase::run(async |test| {
        let root = root_of(DEFAULT_TENANT);
        let mut builder = TestApp::builder(test)
            .member(
                "agent",
                0xa5,
                root,
                &[
                    ("support.tickets.read", Scope::Tenant),
                    ("support.tickets.reply", Scope::Tenant),
                    ("support.tickets.manage", Scope::Tenant),
                ],
            )
            .principal("buyer", Principal::customer(Uuid::from_u128(0xb5)));
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();

        let body = json!({"category": "product", "subject": "Hàng lỗi", "body": "Áo bị rách khi nhận, cửa hàng không đổi.", "name": "An"});
        let (status, opened) = call(&app, "POST", "/public/tickets", "buyer", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        let id = opened["data"]["id"].as_str().unwrap().to_owned();
        let (status, _) =
            call(&app, "POST", &format!("/tickets/{id}/messages"), "agent", Some(json!({"body": "Đã ghi nhận."}))).await;
        assert_eq!(status, StatusCode::CREATED);

        let path = format!("/tickets/{id}/escalate");
        let (status, first) = call(&app, "POST", &path, "agent", Some(json!({"to": "consumer_complaint"}))).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["data"]["resolution"], "escalated");
        let (status, again) = call(&app, "POST", &path, "agent", Some(json!({"to": "consumer_complaint"}))).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["case_id"], first["data"]["case_id"]);

        let db = app.database().read();
        let sql = format!(
            "SELECT kind, channel FROM compliance_cases WHERE target = 'support.ticket:{id}'"
        );
        let rows = db.query_all_raw(Statement::from_string(db.get_database_backend(), sql)).await.unwrap();
        assert_eq!(rows.len(), 1, "exactly one complaint case");
        let kind: String = rows[0].try_get("", "kind").unwrap();
        assert_eq!(kind, "consumer_complaint");
    })
    .await
    .unwrap();
}
