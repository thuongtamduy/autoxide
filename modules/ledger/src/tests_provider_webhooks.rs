//! Test webhook theo khách (bước 70 P7, cả SQLite và Postgres): URL của mỗi khách mang định danh
//! riêng, tìm đúng khách trước khi kiểm chữ ký; hai khách không lẫn nhau; định danh sai bị từ chối;
//! gửi lại không ghi hai lần; URL hợp lệ vẫn cần chữ ký đúng.

use std::sync::Arc;

use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{BankFeed, BankTransaction};
use autoxide_kernel::provider::{ProviderAccounts, WebhookEndpoint};
use autoxide_testkit::{FakeBankFeed, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

const ALL: [&str; 3] = ["ledger.read", "ledger.settings_manage", "kernel.settings.manage"];
const ACCOUNT: &str = "0011001234567";
const FEED: WebhookEndpoint = WebhookEndpoint::new(
    "ledger.connection.fake",
    "/api/v1/ledger/bank-feeds/fake",
    "Paste into the fake bank feed.",
);

async fn setup(test: &TestDatabase) -> TestApp {
    let app = TestApp::builder(test)
        .tenant("one")
        .tenant("two")
        .user_in("one", "one", 1, &ALL)
        .user_in("two", "two", 2, &ALL)
        .contribute::<dyn BankFeed>(Arc::new(FakeBankFeed::new()))
        .module(crate::module())
        .build()
        .await
        .unwrap();
    app.api::<ProviderAccounts>().unwrap().register_webhook(FEED);
    for token in ["one", "two"] {
        let answer = app
            .post("/api/v1/ledger/bank-accounts")
            .token(token)
            .json(json!({"bank_bin": "970436", "account_number": ACCOUNT,
                         "account_name": "CUA HANG", "feed_provider": "fake"}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::CREATED, "{}", answer.text());
    }
    app
}

/// Đường dẫn webhook của khách có token đăng nhập `token`.
async fn webhook_path(app: &TestApp, token: &str) -> String {
    let listed = app.get("/api/v1/kernel/connections/webhooks").token(token).send().await.unwrap();
    assert_eq!(listed.status(), StatusCode::OK, "{}", listed.text());
    let listed = listed.json();
    let rows = listed["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{listed}");
    assert_eq!(rows[0]["connection"], "ledger.connection.fake");
    rows[0]["path"].as_str().unwrap().to_owned()
}

/// Gửi webhook KHÔNG kèm đăng nhập hay header chọn khách — như nhà cung cấp gửi.
async fn push(
    app: &TestApp,
    path: &str,
    txn: &BankTransaction,
    signed: bool,
) -> (StatusCode, Value) {
    let (headers, body) = FakeBankFeed::webhook(std::slice::from_ref(txn));
    let mut request =
        app.request(Method::POST, path).bytes(body).header("content-type", "application/json");
    for (name, value) in &headers {
        request = request.header(name, if signed { value } else { "wrong" });
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

fn credit(id: &str) -> BankTransaction {
    FakeBankFeed::credit(id, ACCOUNT, Money::new(150_000, Currency::VND), "CK")
}

#[tokio::test]
async fn webhook_tim_dung_khach_tu_dinh_danh_trong_url() {
    TestDatabase::run(async |test| {
        let app = setup(test).await;
        let (one, two) = (webhook_path(&app, "one").await, webhook_path(&app, "two").await);
        assert_ne!(one, two);
        assert!(one.starts_with("/api/v1/ledger/bank-feeds/fake/"), "{one}");

        // Khách một nhận; gửi lại không ghi hai lần.
        let (status, body) = push(&app, &one, &credit("T-1"), true).await;
        assert_eq!((status, &body["data"]["stored"]), (StatusCode::OK, &json!(1)), "{body}");
        let (_, body) = push(&app, &one, &credit("T-1"), true).await;
        assert_eq!(body["data"]["stored"], 0);
        // Cùng mã giao dịch ở khách hai là dữ liệu của khách hai: không bị coi là trùng với khách một.
        let (_, body) = push(&app, &two, &credit("T-1"), true).await;
        assert_eq!(body["data"]["stored"], 1);

        // URL đúng vẫn cần chữ ký đúng.
        let (status, body) = push(&app, &one, &credit("T-2"), false).await;
        assert_eq!(
            (status, &body["code"]),
            (StatusCode::UNAUTHORIZED, &json!("payment.signature_invalid"))
        );

        // Định danh sai: đổi ký tự cuối; ghép ID khách hai với phần thô của khách một; đường khác.
        let flipped =
            format!("{}{}", &one[..one.len() - 1], if one.ends_with('A') { 'B' } else { 'A' });
        let raw_of_one = one.rsplit(['/', '.']).next().unwrap();
        let crossed = format!("{}.{raw_of_one}", two.rsplit_once('.').unwrap().0);
        let other_route = one.replace("/bank-feeds/", "/webhooks/");
        for path in [flipped, crossed, other_route, "/api/v1/ledger/bank-feeds/fake/x".to_owned()] {
            let (status, body) = push(&app, &path, &credit("T-3"), true).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
            assert_eq!(body["code"], "provider.webhook_unknown", "{path}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tool_dat_bo_va_liet_ke_webhook() {
    TestDatabase::run(async |test| {
        let app = setup(test).await;
        let mcp = app.mcp("one").await.unwrap();
        let listed = mcp
            .call_tool(autoxide_testkit::tool("kernel.list_connection_webhooks", json!({})))
            .await
            .unwrap();
        let text = format!("{listed:?}");
        assert!(text.contains("/api/v1/ledger/bank-feeds/fake/"), "{text}");
        // Tên không module nào nhận: lỗi có `code`, như REST.
        let refused = mcp
            .call_tool(autoxide_testkit::tool(
                "kernel.set_setting",
                json!({"name": "ledger.connection.fake.key", "value": "x"}),
            ))
            .await;
        assert!(format!("{refused:?}").contains("tenant_setting_unknown"), "{refused:?}");
    })
    .await
    .unwrap();
}
