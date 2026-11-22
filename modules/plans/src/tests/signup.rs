//! Khách tự đăng ký (bước 35 mục 5, D-349) trên module `plans` (không có `compliance`): đăng ký công
//! khai → mail → xác nhận → cấp phát qua đường của nhân sự vận hành → `completed`; kiểm đầu vào gom
//! lỗi một lần; gửi lại cùng email + slug chỉ giữ một dòng; token sai / hết hạn; hai người cùng
//! slug; danh sách cho nhân sự nền tảng. Cả SQLite và Postgres. Luồng có định danh nằm ở
//! `crates/app/src/e2e/signup.rs` (cần module `compliance` thật).

use std::sync::Arc;

use autoxide_auth::api::Invitations;
use autoxide_kernel::mail::Mailer;
use autoxide_testkit::FakeMailer;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::lifecycle::{call, ok};
use super::provisioning::FakeInvitations;

/// Đăng ký thuộc khách nền tảng: request công khai nêu khách (hay gọi tới tên miền của nó).
const TENANT: &str = "autoxide-tenant";

struct Setup {
    app: TestApp,
    mailer: Arc<FakeMailer>,
    invitations: Arc<FakeInvitations>,
}

const OPS: [&str; 3] = ["plans.plans.manage", "plans.subscriptions.read", "kernel.tenants.read"];

async fn setup_with(test: &TestDatabase, enabled: bool) -> Setup {
    let mailer = Arc::new(FakeMailer::default());
    let invitations = Arc::new(FakeInvitations::default());
    let dyn_mailer: Arc<dyn Mailer> = mailer.clone();
    let dyn_invitations: Arc<dyn Invitations> = invitations.clone();
    let app = TestApp::builder(test)
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .setting("AUTOXIDE_PLANS_SIGNUP", if enabled { "true" } else { "false" })
        .setting("AUTOXIDE_PLANS_SIGNUP_URL", "https://www.example.com/signup?token={token}")
        // Không chờ 3 giây giữa lấy token form và gửi.
        .setting("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0")
        .provide::<dyn Mailer>(dyn_mailer)
        .provide::<dyn Invitations>(dyn_invitations)
        .module(crate::module())
        .tenant("beta")
        .user("ops", 1, &OPS)
        .user("clerk", 2, &[])
        .user_in("beta", "pretender", 3, &OPS)
        .build()
        .await
        .unwrap();
    Setup { app, mailer, invitations }
}

async fn setup(test: &TestDatabase) -> Setup {
    setup_with(test, true).await
}

/// Gói đang bán `code`; `extra` ghi đè trường.
async fn plan(app: &TestApp, code: &str, extra: Value) -> String {
    let mut body = json!({"code": code, "name": code, "status": "active", "self_signup": true});
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            body.insert(key.clone(), value.clone());
        }
    }
    let created = ok(app, Method::POST, "/plans/plans", "ops", Some(body)).await;
    created["data"]["id"].as_str().unwrap().to_owned()
}

/// Gửi yêu cầu đăng ký công khai (không đăng nhập), kèm token form mới.
async fn sign_up(app: &TestApp, mut body: Value) -> (StatusCode, Value) {
    let issued = app
        .get("/api/v1/antispam/token?scope=plans:signup")
        .header(TENANT, "default")
        .send()
        .await
        .unwrap();
    assert_eq!(issued.status(), StatusCode::OK, "{}", issued.text());
    body["form_token"] = issued.json()["data"]["token"].clone();
    let answer = app
        .post("/api/v1/plans/signups")
        .header(TENANT, "default")
        .json(body)
        .send()
        .await
        .unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

fn body(email: &str, slug: &str, plan: &str) -> Value {
    json!({"email": email, "tenant_name": "Acme Ltd", "slug": slug, "plan_code": plan, "accept_terms": true, "admin_name": "Owner"})
}

async fn public(
    app: &TestApp,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, &format!("/api/v1/plans{uri}")).header(TENANT, "default");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap();
    for _ in 0..10 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

/// Token trong mail cuối gửi tới `email`.
fn token_for(setup: &Setup, email: &str) -> String {
    let mails = setup.mailer.sent();
    let mail = mails.iter().rev().find(|mail| mail.to == email).expect("no mail");
    let start = mail.text.find("token=").expect("no link") + "token=".len();
    mail.text[start..].split_whitespace().next().unwrap().to_owned()
}

fn mails_to(setup: &Setup, email: &str) -> usize {
    setup.mailer.sent().iter().filter(|mail| mail.to == email).count()
}

#[tokio::test]
async fn tu_dang_ky_xac_nhan_email_roi_cap_phat_khach() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        plan(app, "starter", json!({})).await;
        let (status, accepted) = sign_up(app, body("Owner@Acme.example", "acme", "starter")).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
        assert_eq!(accepted["data"]["accepted"], true);
        drain(app).await;
        let token = token_for(&setup, "owner@acme.example");

        let (status, confirmed) =
            public(app, Method::POST, "/signups/confirm", Some(json!({"token": token}))).await;
        assert_eq!(status, StatusCode::OK, "{confirmed}");
        assert_eq!(confirmed["data"]["status"], "provisioning");
        assert!(confirmed["data"]["confirmed_at"].is_string());
        let id = confirmed["data"]["id"].as_str().unwrap().to_owned();
        drain(app).await;

        let (status, got) =
            public(app, Method::GET, &format!("/signups/{id}?token={token}"), None).await;
        assert_eq!(status, StatusCode::OK, "{got}");
        assert_eq!(got["data"]["status"], "completed", "{got}");
        let tenant = got["data"]["tenant_id"].as_str().unwrap().to_owned();
        let row = ok(app, Method::GET, &format!("/kernel/tenants/{tenant}"), "ops", None).await;
        assert_eq!(row["data"]["slug"], "acme");
        // Người đăng ký được mời làm quản trị viên khách mới.
        let sent = setup.invitations.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(
            (sent[0].1.as_str(), sent[0].2.clone()),
            ("owner@acme.example", vec!["admin".to_owned()])
        );

        // Gọi lại: vô hại, trả trạng thái hiện tại.
        let (status, again) =
            public(app, Method::POST, "/signups/confirm", Some(json!({"token": token}))).await;
        assert_eq!((status, again["data"]["status"].as_str()), (StatusCode::OK, Some("completed")));
        // Token sai: 400 khi xác nhận, 404 khi xem.
        let (status, wrong) =
            public(app, Method::POST, "/signups/confirm", Some(json!({"token": "nope"}))).await;
        assert_eq!(
            (status, wrong["code"].as_str(), wrong["reason"].as_str()),
            (StatusCode::BAD_REQUEST, Some("plans.signup_token_invalid"), Some("unknown"))
        );
        let (status, hidden) =
            public(app, Method::GET, &format!("/signups/{id}?token=nope"), None).await;
        assert_eq!(
            (status, hidden["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("plans.signup_not_found"))
        );

        // Nhân sự nền tảng xem danh sách; người khác thì không.
        let list = ok(app, Method::GET, "/plans/signups?status=completed", "ops", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        let (status, _) = call(app, Method::GET, "/plans/signups", "clerk", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, denied) = call(app, Method::GET, "/plans/signups", "pretender", None).await;
        assert_eq!(
            (status, denied["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("tenant_platform_only"))
        );
        let (status, _) = call(app, Method::GET, "/plans/signups?status=nope", "ops", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tu_dang_ky_kiem_dau_vao_gom_loi_mot_lan() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        plan(app, "starter", json!({})).await;
        plan(app, "closed", json!({"self_signup": false})).await;
        plan(app, "market", json!({"requires_kyc": true})).await;
        let bad = json!({"email": "nope", "tenant_name": "", "slug": "Bad Slug", "plan_code": "closed", "accept_terms": false, "locale": "fr"});
        let (status, problem) = sign_up(app, bad).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let mut fields: Vec<&str> =
            problem["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap()).collect();
        fields.sort_unstable();
        assert_eq!(fields, ["accept_terms", "email", "locale", "plan_code", "slug", "tenant_name"]);
        // Slug của khách đã có.
        let (status, taken) = sign_up(app, body("a@example.com", "beta", "starter")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{taken}");
        assert_eq!((taken["errors"][0]["field"].as_str(), taken["errors"][0]["code"].as_str()), (Some("slug"), Some("taken")));
        // Gói cần định danh mà máy chủ không có `compliance`.
        let (status, kyc) = sign_up(app, body("a@example.com", "fresh", "market")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{kyc}");
        assert_eq!(kyc["errors"][0]["code"], "unavailable");
        // Không ghi gì, không gửi mail.
        drain(app).await;
        assert!(setup.mailer.sent().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tat_dang_ky_thi_404() {
    TestDatabase::run(async |test| {
        let setup = setup_with(test, false).await;
        let app = &setup.app;
        plan(app, "starter", json!({})).await;
        let (status, problem) = sign_up(app, body("a@example.com", "acme", "starter")).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("plans.signup_disabled"))
        );
        let (status, problem) =
            public(app, Method::POST, "/signups/confirm", Some(json!({"token": "x"}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("plans.signup_disabled"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gui_lai_giu_mot_dong_token_cu_het_dung_va_het_han() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        plan(app, "starter", json!({})).await;
        sign_up(app, body("a@example.com", "acme", "starter")).await;
        drain(app).await;
        let first = token_for(&setup, "a@example.com");
        sign_up(app, body("a@example.com", "acme", "starter")).await;
        drain(app).await;
        let second = token_for(&setup, "a@example.com");
        assert_eq!(mails_to(&setup, "a@example.com"), 2);
        assert_ne!(first, second);
        let list = ok(app, Method::GET, "/plans/signups", "ops", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");
        let (status, old) =
            public(app, Method::POST, "/signups/confirm", Some(json!({"token": first}))).await;
        assert_eq!(
            (status, old["code"].as_str()),
            (StatusCode::BAD_REQUEST, Some("plans.signup_token_invalid"))
        );

        // Quá hạn: token không dùng được nữa; lịch hằng giờ đánh dấu `expired`.
        sql(app, "UPDATE plans_signup SET expires_at = '2000-01-01T00:00:00Z'").await;
        let (status, expired) =
            public(app, Method::POST, "/signups/confirm", Some(json!({"token": second}))).await;
        assert_eq!(
            (status, expired["reason"].as_str()),
            (StatusCode::BAD_REQUEST, Some("expired")),
            "{expired}"
        );
        // Lịch đã chạy ở lượt worker trước: xoá dấu để lượt sau chạy lại.
        sql(app, "DELETE FROM plans_jobs").await;
        drain(app).await;
        let list = ok(app, Method::GET, "/plans/signups?status=expired", "ops", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_nguoi_cung_slug_nguoi_den_sau_bi_tu_choi() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        plan(app, "starter", json!({})).await;
        sign_up(app, body("a@example.com", "acme", "starter")).await;
        sign_up(app, body("b@example.com", "acme", "starter")).await;
        drain(app).await;
        let (a, b) = (token_for(&setup, "a@example.com"), token_for(&setup, "b@example.com"));
        let (_, first) =
            public(app, Method::POST, "/signups/confirm", Some(json!({"token": a}))).await;
        assert_eq!(first["data"]["status"], "provisioning");
        let (_, second) =
            public(app, Method::POST, "/signups/confirm", Some(json!({"token": b}))).await;
        assert_eq!(
            (second["data"]["status"].as_str(), second["data"]["reason"].as_str()),
            (Some("rejected"), Some("tenant_slug_taken")),
            "{second}"
        );
    })
    .await
    .unwrap();
}

async fn sql(app: &TestApp, text: &str) {
    use sea_orm::{ConnectionTrait, Statement};
    let control = app.registry().control().write();
    control
        .execute_raw(Statement::from_string(control.get_database_backend(), text))
        .await
        .unwrap();
}
