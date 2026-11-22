//! Test bộ nối SePay với máy chủ giả (cả SQLite và Postgres): xác thực webhook theo tài liệu bằng
//! secret của từng khách, kéo giao dịch theo con trỏ, lỗi của SePay, thử kết nối. Luồng trọn với
//! `ledger` (địa chỉ webhook theo khách, khớp thanh toán VietQR) ở `crates/app/src/e2e/sepay.rs`.

use autoxide_kernel::connections::ConnectionStatus;
use autoxide_kernel::payment::{BankAccountRef, BankFeed};
use autoxide_kernel::provider::{ProviderAccounts, hex, hmac_sha256};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};
use axum::http::{HeaderMap, Method, StatusCode};
use serde_json::{Value, json};

use crate::config::SepayConfig;
use crate::feed::SepayFeed;

const ALL: [&str; 1] = ["kernel.settings.manage"];
const ACCOUNT: &str = "1017588888";

async fn app(test: &TestDatabase, server: &FakeProviderServer) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_SEPAY_API_URL", &format!("{}/v2", server.url()))
        .tenant("one")
        .tenant("two")
        .user_in("one", "one", 1, &ALL)
        .user_in("two", "two", 2, &ALL)
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn call(
    app: &TestApp,
    token: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, uri).token(token);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

/// Khách `token` đặt một trường của tài khoản SePay qua API cài đặt.
async fn set(app: &TestApp, token: &str, field: &str, value: &str) {
    let uri = format!("/api/v1/kernel/settings/sepay.connection.bank.{field}");
    let (status, body) = call(app, token, Method::PUT, &uri, Some(json!({"value": value}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Secret không bao giờ được trả lại.
    assert_eq!((&body["data"]["is_set"], &body["data"]["value"]), (&json!(true), &Value::Null));
    assert!(!body.to_string().contains(value), "{body}");
}

/// Thân webhook theo payload mẫu của tài liệu.
fn payload(id: i64, amount: i64, content: &str, reference: &str) -> Vec<u8> {
    json!({
        "id": id, "gateway": "Vietcombank", "transactionDate": "2024-07-02 11:08:33",
        "accountNumber": ACCOUNT, "subAccount": "", "code": null, "content": content,
        "transferType": "in", "description": content, "transferAmount": amount,
        "accumulated": 105_000_000, "referenceCode": reference
    })
    .to_string()
    .into_bytes()
}

/// Header HMAC như SePay ký: `sha256=hex(HMAC(secret, "{timestamp}.{thân thô}"))`.
fn signed(secret: &str, timestamp: i64, body: &[u8]) -> Vec<(&'static str, String)> {
    let mut message = format!("{timestamp}.").into_bytes();
    message.extend_from_slice(body);
    vec![
        ("X-SePay-Signature", format!("sha256={}", hex(&hmac_sha256(secret.as_bytes(), &message)))),
        ("X-SePay-Timestamp", timestamp.to_string()),
    ]
}

fn now() -> i64 {
    Timestamp::now().as_unix_seconds()
}

fn feed(app: &TestApp, server: &FakeProviderServer) -> SepayFeed {
    let accounts: ProviderAccounts = (*app.api::<ProviderAccounts>().unwrap()).clone();
    let config = SepayConfig { api_url: format!("{}/v2", server.url()) };
    SepayFeed::new(accounts, ConnectionStatus, config).unwrap()
}

fn headers(pairs: &[(&'static str, String)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(*name, value.parse().unwrap());
    }
    headers
}

#[tokio::test]
async fn xac_thuc_webhook_bang_secret_cua_dung_khach() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server).await;
        set(&app, "one", "webhook_secret", "secret-of-one").await;
        let feed = feed(&app, &server);
        let (one, two) = (app.tenant("one").await.unwrap(), app.tenant("two").await.unwrap());
        let body = payload(92704, 5_000_000, "SEVN63DC8E5C chuyen tien", "FT24012345678");

        // Khách hai chưa đặt gì: không nhận webhook không xác thực, không dùng secret của khách một.
        let signed_by_one = headers(&signed("secret-of-one", now(), &body));
        let missing = feed.verify_webhook(two.id(), &signed_by_one, &body).await.unwrap_err();
        assert_eq!(missing.code(), "provider.not_configured");
        set(&app, "two", "webhook_api_key", "key-of-two").await;

        // Khách một dùng HMAC-SHA256: chữ ký đúng, còn hạn.
        let found = feed
            .verify_webhook(one.id(), &headers(&signed("secret-of-one", now(), &body)), &body)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].amount.minor(), 5_000_000);
        assert_eq!(found[0].account_number, ACCOUNT);
        assert_eq!(feed.webhook_ack().unwrap().body, r#"{"success": true}"#);

        let mut wrong = signed("secret-of-one", now(), &body);
        wrong[0].1 = format!("sha256={}", "0".repeat(64));
        let api_key = vec![("authorization", "Apikey key-of-two".to_owned())];
        for (case, pairs) in [
            ("chữ ký sai", wrong),
            ("thiếu header", Vec::new()),
            ("ký quá 5 phút trước", signed("secret-of-one", now() - 301, &body)),
            ("ký bằng secret của khách khác", signed("secret-of-two", now(), &body)),
            ("API key không thay được chữ ký", api_key.clone()),
        ] {
            let refused = feed.verify_webhook(one.id(), &headers(&pairs), &body).await.unwrap_err();
            assert_eq!(refused.code(), "payment.signature_invalid", "{case}");
        }
        // Thân bị sửa sau khi ký.
        let other = payload(92704, 9_000_000, "SEVN63DC8E5C chuyen tien", "FT24012345678");
        let tampered = headers(&signed("secret-of-one", now(), &body));
        let refused = feed.verify_webhook(one.id(), &tampered, &other).await.unwrap_err();
        assert_eq!(refused.code(), "payment.signature_invalid");

        // Khách hai dùng API Key: đúng dạng `Authorization: Apikey <key>`.
        assert_eq!(
            feed.verify_webhook(two.id(), &headers(&api_key), &body).await.unwrap().len(),
            1
        );
        for bad in ["Apikey secret-of-one", "Bearer key-of-two", "key-of-two", "Apikey "] {
            let pairs = vec![("authorization", bad.to_owned())];
            let refused = feed.verify_webhook(two.id(), &headers(&pairs), &body).await.unwrap_err();
            assert_eq!(refused.code(), "payment.signature_invalid", "{bad}");
        }
        // Thân không phải JSON của tài liệu (webhook đặt định dạng form).
        let form = b"id=1&transferAmount=5";
        let refused = feed.verify_webhook(two.id(), &headers(&api_key), form).await.unwrap_err();
        assert_eq!(refused.code(), "payment.provider_rejected");

        // Địa chỉ webhook của khách có trong danh sách kết nối.
        set(&app, "one", "api_token", "token-of-one").await;
        let (status, listed) =
            call(&app, "one", Method::GET, "/api/v1/kernel/connections/webhooks", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(
            listed["data"][0]["path"]
                .as_str()
                .unwrap()
                .starts_with("/api/v1/ledger/bank-feeds/sepay/"),
            "{listed}"
        );
    })
    .await
    .unwrap();
}

fn api_transaction(id: &str, amount: i64, reference: &str) -> Value {
    json!({
        "id": id, "transaction_date": "2026-11-20 09:35:12", "account_number": ACCOUNT,
        "va": null, "transfer_type": "in", "amount_in": amount, "amount_out": 0,
        "accumulated": 0, "transaction_content": "Thanh toan", "reference_number": reference,
        "code": null, "bank_brand_name": "Vietcombank",
        "bank_account_id": "f9e8d7c6-b5a4-3210-fedc-ba0987654321", "va_id": null,
        "webhook_success": 1
    })
}

fn page(transactions: Vec<Value>, has_more: bool) -> Value {
    json!({"status": "success", "data": transactions,
           "meta": {"pagination": {"total": 3, "per_page": 100, "current_page": 1,
                                   "last_page": 2, "has_more": has_more}}})
}

#[tokio::test]
async fn keo_giao_dich_theo_con_tro_bang_token_cua_tung_khach() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server).await;
        set(&app, "one", "api_token", "token-of-one").await;
        set(&app, "two", "api_token", "token-of-two").await;
        let feed = feed(&app, &server);
        let (one, two) = (app.tenant("one").await.unwrap(), app.tenant("two").await.unwrap());
        let account = BankAccountRef::new("970436", ACCOUNT);

        server.reply(
            "GET",
            "/v2/bank-accounts",
            200,
            json!({"status": "success", "data": [
                {"id": "00000000-0000-0000-0000-00000000aaaa", "account_number": "10175888889"},
                {"id": "f9e8d7c6-b5a4-3210-fedc-ba0987654321", "account_number": ACCOUNT}]}),
        );
        server.reply(
            "GET",
            "/v2/transactions",
            200,
            page(vec![api_transaction("t-1", 1_000, "FT1")], true),
        );
        server.reply(
            "GET",
            "/v2/transactions",
            200,
            page(
                vec![api_transaction("t-2", 2_000, "FT2"), api_transaction("t-3", 3_000, "")],
                false,
            ),
        );

        // Trang một: còn trang sau, con trỏ giữ mốc và sang trang hai.
        let first = feed.fetch(one.id(), &account, None).await.unwrap();
        assert!(first.has_more);
        assert_eq!(first.transactions.len(), 1);
        assert_eq!(first.transactions[0].amount.minor(), 1_000);
        let cursor = first.next_cursor.clone().unwrap();
        assert!(cursor.starts_with("from|") && cursor.ends_with("|2"), "{cursor}");
        let seen = server.take_requests();
        assert_eq!(seen.len(), 2);
        assert!(
            seen.iter()
                .all(|request| request.header("authorization") == Some("Bearer token-of-one"))
        );
        assert_eq!(
            (seen[0].path.as_str(), seen[0].query_param("q")),
            ("/v2/bank-accounts", Some(ACCOUNT))
        );
        let listed = &seen[1];
        assert_eq!(
            listed.query_param("bank_account_id"),
            Some("f9e8d7c6-b5a4-3210-fedc-ba0987654321")
        );
        assert_eq!(listed.query_param("transaction_date_sort"), Some("asc"));
        assert_eq!(
            (listed.query_param("per_page"), listed.query_param("page")),
            (Some("100"), Some("1"))
        );
        assert!(
            listed.query_param("transaction_date_from").is_some()
                && listed.query_param("since_id").is_none()
        );

        // Trang hai là trang cuối: mốc kế là giao dịch cuối cùng.
        let second = feed.fetch(one.id(), &account, Some(cursor)).await.unwrap();
        assert!(!second.has_more);
        assert_eq!(second.transactions.len(), 2);
        // Không có mã tham chiếu thì mã chống trùng là `id` của SePay.
        assert_eq!(second.transactions[1].provider_txn_id, "tx:t-3");
        assert_eq!(second.next_cursor.as_deref(), Some("since|t-3|1"));
        assert_eq!(server.take_requests()[1].query_param("page"), Some("2"));

        // Lần sau đọc từ `since_id`; khách hai dùng token của khách hai.
        let third = feed.fetch(two.id(), &account, second.next_cursor.clone()).await.unwrap();
        assert_eq!(third.transactions.len(), 2);
        let seen = server.take_requests();
        assert!(
            seen.iter()
                .all(|request| request.header("authorization") == Some("Bearer token-of-two"))
        );
        assert_eq!(seen[1].query_param("since_id"), Some("t-3"));
        assert!(seen[1].query_param("transaction_date_from").is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loi_cua_sepay_va_thu_ket_noi() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server).await;
        let test_uri = "/api/v1/kernel/settings/sepay.connection.bank.api_token/test";

        // Chưa đặt token: thử kết nối báo rõ thiếu gì, không gọi ra ngoài.
        let (status, body) = call(&app, "one", Method::POST, test_uri, None).await;
        assert_eq!((status, &body["data"]["ok"]), (StatusCode::OK, &json!(false)), "{body}");
        assert!(
            body["data"]["error"].as_str().unwrap().contains("sepay.connection.bank.api_token")
        );
        assert!(server.take_requests().is_empty());

        set(&app, "one", "api_token", "token-of-one").await;
        server.reply("GET", "/v2/bank-accounts", 200, json!({"status": "success", "data": []}));
        server.reply(
            "GET",
            "/v2/bank-accounts",
            401,
            json!({"status": "error", "message": "Invalid API key", "error_code": "unauthorized"}),
        );
        let (_, body) = call(&app, "one", Method::POST, test_uri, None).await;
        assert_eq!(body["data"]["ok"], true, "{body}");
        assert_eq!(body["data"]["connection"], "sepay.connection.bank");
        assert_eq!(server.take_requests()[0].query_param("per_page"), Some("1"));
        // Token bị từ chối: báo lỗi, không lộ token.
        let (_, body) = call(&app, "one", Method::POST, test_uri, None).await;
        assert_eq!(body["data"]["ok"], false, "{body}");
        assert!(!body.to_string().contains("token-of-one"), "{body}");
        let (_, listed) =
            call(&app, "one", Method::GET, "/api/v1/kernel/settings?group=connection", None).await;
        let row = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == "sepay.connection.bank.api_token")
            .unwrap();
        assert_eq!(
            (&row["secret"], &row["is_set"], &row["value"]),
            (&json!(true), &json!(true), &Value::Null)
        );
        assert!(row["last_error"].as_str().is_some(), "{row}");

        // Kéo: tài khoản không thuộc công ty SePay của khách; SePay quá tải (429).
        let feed = feed(&app, &server);
        let one = app.tenant("one").await.unwrap();
        let two = app.tenant("two").await.unwrap();
        let account = BankAccountRef::new("970436", ACCOUNT);
        // Khách hai chỉ dùng webhook (không có token): lịch kéo không lỗi, không gọi ra ngoài.
        server.take_requests();
        let nothing = feed.fetch(two.id(), &account, Some("since|t-9|1".to_owned())).await.unwrap();
        assert!(nothing.transactions.is_empty() && !nothing.has_more);
        assert_eq!(nothing.next_cursor.as_deref(), Some("since|t-9|1"));
        assert!(server.take_requests().is_empty());
        server.reply("GET", "/v2/bank-accounts", 200, json!({"status": "success", "data": []}));
        // (hàng đợi còn câu 401 ở đầu: bỏ nó bằng một lời gọi)
        let refused = feed.fetch(one.id(), &account, None).await.unwrap_err();
        assert_eq!(refused.code(), "payment.provider_rejected");
        let unlinked = feed.fetch(one.id(), &account, None).await.unwrap_err();
        assert_eq!(unlinked.code(), "payment.provider_rejected");
        server.reply(
            "GET",
            "/v2/bank-accounts",
            429,
            json!({"status": "error", "error_code": "rate_limited"}),
        );
        let _ = feed.fetch(one.id(), &account, None).await;
        let busy = feed.fetch(one.id(), &account, None).await.unwrap_err();
        assert_eq!(busy.code(), "payment.provider_unavailable");
    })
    .await
    .unwrap();
}
