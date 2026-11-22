//! Bộ nối SePay với `ledger` thật (bước 70 P7; cả SQLite và Postgres): webhook của SePay tới địa chỉ
//! riêng của từng khách, chữ ký theo tài liệu, gửi lại không ghi trùng, hai khách hai tài khoản không
//! lẫn nhau, thanh toán VietQR tự khớp theo mã và số tiền như luồng sẵn có của `ledger`.

use autoxide_kernel::provider::{hex, hmac_sha256};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

const ALL: [&str; 5] = [
    "ledger.read",
    "ledger.settings_manage",
    "ledger.payments_create",
    "ledger.reconcile",
    "kernel.settings.manage",
];
const ACCOUNT: &str = "1017588888";

async fn app(test: &TestDatabase, server: &FakeProviderServer) -> TestApp {
    TestApp::builder(test)
        .setting("AUTOXIDE_SEPAY_API_URL", &format!("{}/v2", server.url()))
        .tenant("one")
        .tenant("two")
        .user_in("one", "one", 1, &ALL)
        .user_in("two", "two", 2, &ALL)
        .module(autoxide_sepay::module())
        .module(autoxide_ledger::module())
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

async fn bank_account(app: &TestApp, token: &str) {
    let (status, body) = call(
        app,
        token,
        Method::POST,
        "/api/v1/ledger/bank-accounts",
        Some(json!({"bank_bin": "970436", "account_number": ACCOUNT, "account_name": "CUA HANG",
                    "feed_provider": "sepay"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

async fn webhook_path(app: &TestApp, token: &str) -> String {
    let (status, body) =
        call(app, token, Method::GET, "/api/v1/kernel/connections/webhooks", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["connection"] == "sepay.connection.bank")
        .unwrap_or_else(|| panic!("{body}"));
    row["path"].as_str().unwrap().to_owned()
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

/// Gửi như SePay gửi: không đăng nhập, không header chọn khách.
async fn push(
    app: &TestApp,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> (StatusCode, String) {
    let mut request = app
        .request(Method::POST, path)
        .bytes(body.to_vec())
        .header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(name, value);
    }
    // Câu trả lời thành công là dạng SePay yêu cầu, không phải envelope chuẩn.
    let answer = request.unchecked().send().await.unwrap();
    (answer.status(), answer.text())
}

fn now() -> i64 {
    Timestamp::now().as_unix_seconds()
}

async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..5 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

async fn vietqr(app: &TestApp, token: &str, amount: i64) -> Value {
    let (status, body) = call(
        app,
        token,
        Method::POST,
        "/api/v1/ledger/payments/vietqr",
        Some(json!({"amount": {"amount": amount.to_string(), "currency": "VND"},
                    "description": "Order", "source_id": "O-1", "reference": "R-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["data"].clone()
}

async fn payment_status(app: &TestApp, token: &str, id: &str) -> String {
    let uri = format!("/api/v1/ledger/payments/{id}");
    let (_, body) = call(app, token, Method::GET, &uri, None).await;
    body["data"]["status"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn webhook_hmac_khop_thanh_toan_va_hai_khach_khong_lan_nhau() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server).await;
        for (token, secret) in [("one", "secret-of-one"), ("two", "secret-of-two")] {
            set(&app, token, "webhook_secret", secret).await;
            bank_account(&app, token).await;
        }
        let (one, two) = (webhook_path(&app, "one").await, webhook_path(&app, "two").await);
        assert!(one.starts_with("/api/v1/ledger/bank-feeds/sepay/") && one != two, "{one} {two}");

        // Mỗi khách một thanh toán VietQR cùng số tiền.
        let paid = vietqr(&app, "one", 5_000_000).await;
        let other = vietqr(&app, "two", 5_000_000).await;
        let code = paid["transfer_code"].as_str().unwrap();
        let body = payload(92704, 5_000_000, &format!("{code} chuyen tien"), "FT24012345678");

        // Chữ ký của khách một gửi tới địa chỉ của khách hai: bị từ chối, không lưu gì.
        let (status, text) = push(&app, &two, &signed("secret-of-one", now(), &body), &body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
        assert!(text.contains("payment.signature_invalid"), "{text}");

        // Đúng khách, đúng chữ ký: trả đúng câu SePay chờ.
        let (status, text) = push(&app, &one, &signed("secret-of-one", now(), &body), &body).await;
        assert_eq!((status, text.as_str()), (StatusCode::OK, r#"{"success": true}"#));
        // SePay gửi lại (retry / replay): vẫn trả thành công, không ghi lần hai.
        let (status, text) = push(&app, &one, &signed("secret-of-one", now(), &body), &body).await;
        assert_eq!((status, text.as_str()), (StatusCode::OK, r#"{"success": true}"#));

        drain(&app).await;
        assert_eq!(payment_status(&app, "one", paid["id"].as_str().unwrap()).await, "succeeded");
        assert_eq!(payment_status(&app, "two", other["id"].as_str().unwrap()).await, "pending");
        let (_, exceptions) =
            call(&app, "one", Method::GET, "/api/v1/ledger/exceptions?status=open", None).await;
        assert_eq!(exceptions["data"].as_array().unwrap().len(), 0, "{exceptions}");

        // Sai chữ ký, thiếu header, timestamp quá 5 phút, thân bị sửa sau khi ký.
        let fresh = payload(92705, 1_000, "khac", "FT24099999999");
        let mut wrong = signed("secret-of-one", now(), &fresh);
        wrong[0].1 = format!("sha256={}", "0".repeat(64));
        let stale = signed("secret-of-one", now() - 301, &fresh);
        let tampered = signed("secret-of-one", now(), &body);
        for headers in [wrong, Vec::new(), stale, tampered] {
            let (status, text) = push(&app, &one, &headers, &fresh).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
        }
        // API key không thay được chữ ký khi khách đã đặt Secret Key.
        set(&app, "one", "webhook_api_key", "key-of-one").await;
        let (status, _) =
            push(&app, &one, &[("Authorization", "Apikey key-of-one".to_owned())], &fresh).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn webhook_api_key_va_khach_chua_cau_hinh() {
    TestDatabase::run(async |test| {
        let server = FakeProviderServer::start().await.unwrap();
        let app = app(test, &server).await;
        set(&app, "one", "webhook_api_key", "key-of-one").await;
        bank_account(&app, "one").await;
        bank_account(&app, "two").await;
        let (one, two) = (webhook_path(&app, "one").await, webhook_path(&app, "two").await);
        let body = payload(1, 20_000, "CK", "FT1");
        let key = |value: &str| vec![("Authorization", value.to_owned())];

        let (status, text) = push(&app, &one, &key("Apikey key-of-one"), &body).await;
        assert_eq!((status, text.as_str()), (StatusCode::OK, r#"{"success": true}"#));
        for bad in ["Apikey key-of-two", "Bearer key-of-one", "key-of-one", "Apikey "] {
            let (status, text) = push(&app, &one, &key(bad), &body).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{bad}: {text}");
        }
        // Khách hai chưa đặt gì: không nhận webhook không xác thực, không dùng key của khách một.
        let (status, text) = push(&app, &two, &key("Apikey key-of-one"), &body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
        assert!(text.contains("provider.not_configured"), "{text}");
        // Thân không phải JSON của tài liệu (vd webhook đặt định dạng form).
        let (status, text) =
            push(&app, &one, &key("Apikey key-of-one"), b"id=1&transferAmount=5").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
        assert!(text.contains("payment.provider_rejected"), "{text}");
    })
    .await
    .unwrap();
}
