//! Khách tự đăng ký (bước 35 mục 5, D-349) trên chương trình thật: đăng ký công khai → xác nhận email
//! (ghi chấp nhận điều khoản ở `compliance`) → định danh qua nhà cung cấp giả (callback có chữ ký →
//! job của `compliance` → event `compliance.kyc_verified` / `kyc_rejected`) → `plans` cấp phát bằng
//! đường của nhân sự vận hành, mời quản trị viên bằng lời mời thật của `auth`. Chạy trên cả SQLite và
//! Postgres.

use std::sync::Arc;

use autoxide_compliance::api::{AcceptanceGate, Subject};
use autoxide_kernel::identity::IdentityVerifier;
use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_testkit::{FakeIdentityVerifier, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};

const OPS: [&str; 5] = [
    "plans.plans.manage",
    "plans.subscriptions.read",
    "plans.provision",
    "compliance.documents.manage",
    "kernel.tenants.read",
];
/// Đăng ký thuộc khách nền tảng: request công khai nêu khách bằng header.
const TENANT: &str = "autoxide-tenant";
/// Token đặt thẳng vào dòng đăng ký (bản băm), thay cho đọc thư.
const TOKEN: &str = "known-signup-token";

async fn app(test: &TestDatabase, verifier: Arc<FakeIdentityVerifier>) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-signup-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .setting("AUTOXIDE_PLANS_SIGNUP", "true")
        .setting("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0")
        // Trang quay về do người vận hành cấu hình: `compliance` tin địa chỉ `plans` truyền qua `KycGate`, không
        // so với tên miền của khách nền tảng (D-20261128-return-url-kernel) — lượt đăng ký cần KYC không bị 422.
        .setting("AUTOXIDE_PLANS_SIGNUP_RETURN_URL", "https://signup.platform.example/done?id={id}")
        .provide::<dyn IdentityVerifier>(verifier)
        .tenant("beta")
        .user("ops", 1, &OPS);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    uri: &str,
    who: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, &format!("/api/v1{uri}"));
    request = match who {
        Some(who) => request.token(who),
        None => request.header(TENANT, "default"),
    };
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn ok(
    app: &TestApp,
    method: Method,
    uri: &str,
    who: Option<&str>,
    body: Option<Value>,
) -> Value {
    let (status, body) = call(app, method.clone(), uri, who, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {body}");
    body
}

async fn sql(app: &TestApp, text: &str) {
    let control = app.registry().control().write();
    control
        .execute_raw(Statement::from_string(control.get_database_backend(), text))
        .await
        .unwrap();
}

/// Chạy job và phát event tới khi hết việc.
async fn settle(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..8 {
        let jobs = worker.run_once().await.unwrap();
        let mut events = 0;
        loop {
            let sent = dispatcher.run_once().await.unwrap();
            if sent == 0 {
                break;
            }
            events += sent;
        }
        if jobs == 0 && events == 0 {
            break;
        }
    }
}

/// Gói đang bán `code`, mở cho tự đăng ký.
async fn plan(app: &TestApp, code: &str, requires_kyc: bool) {
    let body = json!({"code": code, "name": code, "status": "active", "self_signup": true, "requires_kyc": requires_kyc});
    ok(app, Method::POST, "/plans/plans", Some("ops"), Some(body)).await;
}

/// Đăng ký công khai rồi đặt token đã biết cho dòng vừa tạo; trả kết quả xác nhận.
async fn sign_up_and_confirm(app: &TestApp, email: &str, slug: &str, code: &str) -> Value {
    let issued = ok(app, Method::GET, "/antispam/token?scope=plans:signup", None, None).await;
    let body = json!({"email": email, "tenant_name": "Shop", "slug": slug, "plan_code": code, "accept_terms": true, "party_type": "individual", "form_token": issued["data"]["token"]});
    let (status, accepted) = call(app, Method::POST, "/plans/signups", None, Some(body)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    settle(app).await;
    let hash = autoxide_kernel::secret::sha256(TOKEN);
    sql(app, &format!("UPDATE plans_signup SET token_hash = '{hash}' WHERE slug = '{slug}'")).await;
    ok(app, Method::POST, "/plans/signups/confirm", None, Some(json!({"token": TOKEN}))).await
}

/// Báo kết quả định danh của lượt mới nhất qua callback có chữ ký.
async fn finish_kyc(app: &TestApp, verifier: &FakeIdentityVerifier, verified: bool) {
    let attempt = verifier.started.lock().unwrap().last().unwrap().attempt_id;
    let reference = FakeIdentityVerifier::reference(attempt);
    verifier.complete(&reference, verified, "NGUYEN VAN A", "001099012345");
    let (headers, body) = FakeIdentityVerifier::callback(&reference);
    let mut request = app
        .post("/api/v1/compliance/kyc/callbacks/fake")
        .header(TENANT, "default")
        .header("content-type", "application/json")
        .bytes(body);
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    assert_eq!(request.send().await.unwrap().status(), StatusCode::NO_CONTENT);
    settle(app).await;
}

async fn status_of(app: &TestApp, id: &str) -> Value {
    ok(app, Method::GET, &format!("/plans/signups/{id}?token={TOKEN}"), None, None).await["data"]
        .clone()
}

#[tokio::test]
async fn dinh_danh_xong_thi_cap_phat_khach_va_ghi_chap_nhan_dieu_khoan() {
    TestDatabase::run(async |test| {
        let verifier = Arc::new(FakeIdentityVerifier::new());
        let app = app(test, verifier.clone()).await;
        // Nhà vận hành đã đăng điều khoản sử dụng.
        let doc = ok(
            &app,
            Method::POST,
            "/compliance/documents",
            Some("ops"),
            Some(json!({"kind": "terms_of_use", "title": "Terms"})),
        )
        .await;
        let id = doc["data"]["id"].as_str().unwrap();
        let version = json!({"locale": "en", "title": "Terms", "body": "Version 1"});
        ok(
            &app,
            Method::POST,
            &format!("/compliance/documents/{id}/versions"),
            Some("ops"),
            Some(version),
        )
        .await;
        plan(&app, "market", true).await;

        let confirmed = sign_up_and_confirm(&app, "seller@example.com", "seller", "market").await;
        let signup = &confirmed["data"];
        assert_eq!(signup["status"], "awaiting_kyc", "{confirmed}");
        assert_eq!(signup["kyc"]["kind"], "redirect");
        assert!(signup["kyc"]["url"].as_str().unwrap().starts_with("https://"));
        let id = signup["id"].as_str().unwrap().to_owned();
        // Chấp nhận điều khoản đã ghi cho email ở khách nền tảng.
        let gate = app.api::<dyn AcceptanceGate>().unwrap();
        let subject = Subject::Email("seller@example.com".to_owned());
        gate.require_accepted(DEFAULT_TENANT, subject, "terms_of_use").await.unwrap();
        // Chưa định danh: chưa có khách.
        assert_eq!(status_of(&app, &id).await["status"], "awaiting_kyc");

        finish_kyc(&app, &verifier, true).await;
        let done = status_of(&app, &id).await;
        assert_eq!(done["status"], "completed", "{done}");
        let tenant = done["tenant_id"].as_str().unwrap().to_owned();
        let row =
            ok(&app, Method::GET, &format!("/kernel/tenants/{tenant}"), Some("ops"), None).await;
        assert_eq!(row["data"]["slug"], "seller");
        let provisioning = done["provisioning_id"].as_str().unwrap();
        let run = ok(
            &app,
            Method::GET,
            &format!("/plans/provisionings/{provisioning}"),
            Some("ops"),
            None,
        )
        .await;
        assert_eq!(
            (run["data"]["status"].as_str(), run["data"]["admin_email"].as_str()),
            (Some("done"), Some("seller@example.com")),
            "{run}"
        );
        // Event đến lại: vô hại.
        finish_kyc(&app, &verifier, true).await;
        assert_eq!(status_of(&app, &id).await["status"], "completed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dinh_danh_bi_tu_choi_thi_khong_tao_khach() {
    TestDatabase::run(async |test| {
        let verifier = Arc::new(FakeIdentityVerifier::new());
        let app = app(test, verifier.clone()).await;
        plan(&app, "market", true).await;
        let confirmed = sign_up_and_confirm(&app, "seller@example.com", "seller", "market").await;
        let id = confirmed["data"]["id"].as_str().unwrap().to_owned();
        finish_kyc(&app, &verifier, false).await;
        let rejected = status_of(&app, &id).await;
        assert_eq!(
            (rejected["status"].as_str(), rejected["reason"].as_str()),
            (Some("rejected"), Some("kyc_rejected")),
            "{rejected}"
        );
        assert!(rejected["provisioning_id"].is_null());
        let list = ok(&app, Method::GET, "/plans/provisionings", Some("ops"), None).await;
        assert!(list["data"].as_array().unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn goi_khong_can_dinh_danh_cap_phat_ngay_sau_xac_nhan() {
    TestDatabase::run(async |test| {
        let verifier = Arc::new(FakeIdentityVerifier::new());
        let app = app(test, verifier.clone()).await;
        plan(&app, "starter", false).await;
        let confirmed = sign_up_and_confirm(&app, "owner@example.com", "shop", "starter").await;
        assert_eq!(confirmed["data"]["status"], "provisioning", "{confirmed}");
        assert!(confirmed["data"]["kyc"].is_null());
        let id = confirmed["data"]["id"].as_str().unwrap().to_owned();
        settle(&app).await;
        let done = status_of(&app, &id).await;
        assert_eq!(done["status"], "completed", "{done}");
        assert!(verifier.started.lock().unwrap().is_empty());
        // Nhân sự nền tảng thấy lượt đăng ký; slug đã thành khách thì không đăng ký lại được.
        let list = ok(&app, Method::GET, "/plans/signups", Some("ops"), None).await;
        assert_eq!(list["data"][0]["status"], "completed");
        let issued = ok(&app, Method::GET, "/antispam/token?scope=plans:signup", None, None).await;
        let body = json!({"email": "x@example.com", "tenant_name": "Shop", "slug": "shop", "plan_code": "starter", "accept_terms": true, "form_token": issued["data"]["token"]});
        let (status, taken) = call(&app, Method::POST, "/plans/signups", None, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{taken}");
    })
    .await
    .unwrap();
}
