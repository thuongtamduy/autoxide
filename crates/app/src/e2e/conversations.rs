//! Chat chăm sóc khách trên chương trình thật (cả SQLite và Postgres): người mua nhắn, nhân viên trả
//! lời rồi chuyển cuộc trò chuyện thành phiếu hỗ trợ — `support` thật mở phiếu của người mua, mang lịch
//! sử và liên kết về cuộc trò chuyện (sổ đối tượng); gọi lại không tạo phiếu thứ hai.

use autoxide_kernel::access::{Principal, Scope};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
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
    let mut request = app.request(method, &format!("/api/v1{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

#[tokio::test]
async fn chat_thanh_phieu_cua_support_mot_lan() {
    TestDatabase::run(async |test| {
        let root = root_of(DEFAULT_TENANT);
        let mut builder = TestApp::builder(test)
            .member(
                "agent",
                0xa6,
                root,
                &[
                    ("conversations.read", Scope::Tenant),
                    ("conversations.reply", Scope::Tenant),
                    ("support.tickets.read", Scope::Tenant),
                ],
            )
            .principal("buyer", Principal::customer(Uuid::from_u128(0xb6)));
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();

        let body = json!({"body": "Áo bị rách khi nhận", "client_id": "c-1"});
        let (status, started) =
            call(&app, "POST", "/conversations/me/threads", "buyer", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{started}");
        let id = started["data"]["thread"]["id"].as_str().unwrap().to_owned();
        let body = json!({"body": "Bạn gửi ảnh giúp shop", "client_id": "r-1"});
        let (status, _) = call(
            &app,
            "POST",
            &format!("/conversations/threads/{id}/messages"),
            "agent",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let path = format!("/conversations/threads/{id}/escalate");
        let (status, first) = call(&app, "POST", &path, "agent", None).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let (_, again) = call(&app, "POST", &path, "agent", None).await;
        assert_eq!(again["data"]["ticket_id"], first["data"]["ticket_id"]);

        let ticket = first["data"]["ticket_id"].as_str().unwrap();
        let (status, got) =
            call(&app, "GET", &format!("/support/tickets/{ticket}"), "agent", None).await;
        assert_eq!(status, StatusCode::OK, "{got}");
        assert_eq!(got["data"]["requester"]["user_id"], Uuid::from_u128(0xb6).to_string());
        let text = got.to_string();
        assert!(
            text.contains("Áo bị rách khi nhận") && text.contains("Bạn gửi ảnh giúp shop"),
            "{text}"
        );
        assert!(text.contains(&format!("conversations.thread:{id}")), "{text}");
        let (_, mine) = call(&app, "GET", "/support/me/tickets", "buyer", None).await;
        assert_eq!(mine["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}
