//! Vòng đời thuê bao (bước 35 phần E / F, D-327): mỗi dòng của bảng vòng đời, thu tay, đổi / huỷ /
//! tiếp tục, mail nhắc không gửi trùng, job và lần thu tiền cùng lúc, bộ lọc danh sách khách. Thời
//! gian "trôi" bằng cách sửa mốc trong database rồi chạy worker (job dùng giờ thật).

use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::mail::{Mail, Mailer};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

#[derive(Default)]
struct Outbox(Mutex<Vec<Mail>>);

#[async_trait::async_trait]
impl Mailer for Outbox {
    async fn send(&self, _tenant: &TenantContext, mail: Mail) -> Result<(), Error> {
        self.0.lock().unwrap().push(mail);
        Ok(())
    }
}

const OPS: [&str; 4] = [
    "plans.plans.manage",
    "plans.subscriptions.manage",
    "plans.subscriptions.read",
    "kernel.tenants.read",
];

async fn app(test: &TestDatabase, mails: &Arc<Outbox>) -> TestApp {
    let mailer: Arc<dyn Mailer> = mails.clone();
    TestApp::builder(test)
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .provide::<dyn Mailer>(mailer)
        .module(crate::module())
        .tenant("acme")
        .user("ops", 1, &OPS)
        .user_in("acme", "owner", 2, &["plans.billing.read", "plans.billing.manage"])
        .build()
        .await
        .unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, &format!("/api/v1{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn ok(app: &TestApp, method: Method, uri: &str, who: &str, body: Option<Value>) -> Value {
    let (status, body) = call(app, method.clone(), uri, who, body).await;
    assert!(status.is_success(), "{method} {uri}: {status} {body}");
    body
}

/// Gói `code` giá `amount` VND mỗi tháng (0: không giá), dùng thử `trial` ngày.
async fn priced(app: &TestApp, code: &str, amount: i64, trial: i32) -> (String, Option<String>) {
    let body = json!({"code": code, "name": code, "status": "active", "trial_days": trial, "grace_days": 7, "suspend_days": 30, "retention_days": 90});
    let plan = ok(app, Method::POST, "/plans/plans", "ops", Some(body)).await;
    let id = plan["data"]["id"].as_str().unwrap().to_owned();
    if amount == 0 {
        return (id, None);
    }
    let price = json!({"kind": "recurring", "interval": "month", "amount": {"amount": amount.to_string(), "currency": "VND"}});
    let plan =
        ok(app, Method::POST, &format!("/plans/plans/{id}/prices"), "ops", Some(price)).await;
    let price_id = plan["data"]["prices"][0]["id"].as_str().unwrap().to_owned();
    (id, Some(price_id))
}

async fn acme(app: &TestApp) -> TenantId {
    app.tenant("acme").await.unwrap().id()
}

async fn assign(app: &TestApp, plan: &(String, Option<String>), trial: bool) -> Value {
    let tenant = acme(app).await;
    let body = json!({"plan_id": plan.0, "price_id": plan.1, "trial": trial});
    ok(app, Method::PUT, &format!("/plans/tenants/{tenant}/subscription"), "ops", Some(body)).await
}

async fn sql(app: &TestApp, text: &str) {
    use sea_orm::{ConnectionTrait, Statement};
    let control = app.registry().control().write();
    control
        .execute_raw(Statement::from_string(control.get_database_backend(), text))
        .await
        .unwrap();
}

async fn status(app: &TestApp) -> String {
    let sub = ok(app, Method::GET, "/plans/subscription", "owner", None).await;
    sub["data"]["status"].as_str().unwrap().to_owned()
}

/// Chạy worker tới khi hết job (mỗi lượt chạy tối đa 4 job mỗi module; `plans` có 5 lịch).
async fn tick(app: &TestApp) {
    // Lịch chỉ xếp một job mỗi khung giờ: xoá job đã xong để lượt này xếp lại (thời gian giả).
    sql(app, "DELETE FROM plans_jobs").await;
    let worker = app.modules().worker(4).unwrap();
    for _ in 0..5 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

const PAST: &str = "'2000-01-01T00:00:00Z'";

#[tokio::test]
async fn qua_han_an_han_thanh_tam_khoa_roi_tra_tien_thanh_active() {
    TestDatabase::run(async |test| {
        let mails = Arc::new(Outbox::default());
        let app = app(test, &mails).await;
        let pro = priced(&app, "pro", 199_000, 0).await;
        assign(&app, &pro, false).await;
        assert_eq!(status(&app).await, "active");
        // Hết kỳ, kỳ kế chưa trả: quá hạn, có kỳ thanh toán `renewal` mở.
        sql(&app, &format!("UPDATE plans_subscription SET current_period_end = {PAST}")).await;
        tick(&app).await;
        assert_eq!(status(&app).await, "past_due");
        let periods = ok(&app, Method::GET, "/plans/billing-periods", "owner", None).await;
        let renewal = periods["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["kind"] == "renewal")
            .unwrap()
            .clone();
        assert_eq!(
            (renewal["status"].clone(), renewal["amount"]["amount"].clone()),
            (json!("open"), json!("199000"))
        );
        // Hết ân hạn: tạm khoá, chỉ đọc.
        sql(&app, &format!("UPDATE plans_subscription SET grace_ends_at = {PAST}")).await;
        tick(&app).await;
        assert_eq!(status(&app).await, "suspended");
        let now = ok(&app, Method::GET, "/plans/entitlements", "owner", None).await;
        assert_eq!(now["data"]["access"], "read_only");
        // Danh sách khách lọc theo trạng thái thuê bao (bước 33 + 35).
        let list =
            ok(&app, Method::GET, "/kernel/tenants?subscription_status=suspended", "ops", None)
                .await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");
        // Thu tay: đang dùng lại.
        let id = renewal["id"].as_str().unwrap();
        let body = json!({"provider_reference": "VCB-123"});
        ok(
            &app,
            Method::POST,
            &format!("/plans/billing-periods/{id}/mark-paid"),
            "ops",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status(&app).await, "active");
        let (code, again) = call(
            &app,
            Method::POST,
            &format!("/plans/billing-periods/{id}/mark-paid"),
            "ops",
            Some(body),
        )
        .await;
        assert_eq!(
            (code, again["code"].as_str()),
            (StatusCode::CONFLICT, Some("plans.billing_period_not_payable"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tam_khoa_qua_han_thanh_huy_roi_qua_thoi_han_luu_thanh_xoa() {
    TestDatabase::run(async |test| {
        let mails = Arc::new(Outbox::default());
        let app = app(test, &mails).await;
        let pro = priced(&app, "pro", 199_000, 0).await;
        assign(&app, &pro, false).await;
        sql(
            &app,
            &format!("UPDATE plans_subscription SET status = 'suspended', suspended_at = {PAST}"),
        )
        .await;
        tick(&app).await;
        let sub = ok(&app, Method::GET, "/plans/subscription", "owner", None).await;
        assert_eq!(sub["data"]["status"], "canceled");
        assert!(sub["data"]["delete_after"].is_string());
        // Đăng ký lại trước hạn xoá được (kỳ đầu phải trả).
        sql(&app, &format!("UPDATE plans_subscription SET delete_after = {PAST}")).await;
        tick(&app).await;
        let tenant = acme(&app).await;
        let sub =
            ok(&app, Method::GET, &format!("/plans/tenants/{tenant}/subscription"), "ops", None)
                .await;
        assert_eq!(sub["data"]["status"], "deleted");
        // Khách bị khoá hẳn, dữ liệu giữ lại.
        let (code, body) = call(&app, Method::GET, "/plans/entitlements", "owner", None).await;
        assert_eq!(
            (code, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("tenant_suspended")),
            "{body}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dung_thu_het_han_gia_0_thanh_active_va_huy_cuoi_ky_roi_tiep_tuc() {
    TestDatabase::run(async |test| {
        let mails = Arc::new(Outbox::default());
        let app = app(test, &mails).await;
        let free = priced(&app, "free", 0, 14).await;
        assign(&app, &free, true).await;
        assert_eq!(status(&app).await, "trialing");
        sql(&app, &format!("UPDATE plans_subscription SET current_period_end = {PAST}")).await;
        tick(&app).await;
        assert_eq!(status(&app).await, "active");
        // Khách huỷ cuối kỳ rồi đổi ý.
        let sub =
            ok(&app, Method::POST, "/plans/subscription/cancel", "owner", Some(json!({}))).await;
        assert_eq!(sub["data"]["cancel_at_period_end"], true);
        let sub = ok(&app, Method::POST, "/plans/subscription/resume", "owner", None).await;
        assert_eq!(sub["data"]["cancel_at_period_end"], false);
        // Huỷ cuối kỳ thật: hết kỳ thì huỷ.
        ok(
            &app,
            Method::POST,
            "/plans/subscription/cancel",
            "owner",
            Some(json!({"at_period_end": true})),
        )
        .await;
        sql(&app, &format!("UPDATE plans_subscription SET current_period_end = {PAST}")).await;
        tick(&app).await;
        assert_eq!(status(&app).await, "canceled");
        // Đăng ký lại trước hạn xoá.
        let sub = ok(&app, Method::POST, "/plans/subscription/resume", "owner", None).await;
        assert_eq!(sub["data"]["status"], "active");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ha_goi_doi_het_ky_nang_goi_lap_ky_ti_le() {
    TestDatabase::run(async |test| {
        let mails = Arc::new(Outbox::default());
        let app = app(test, &mails).await;
        let pro = priced(&app, "pro", 300_000, 0).await;
        let basic = priced(&app, "basic", 0, 0).await;
        let max = priced(&app, "max", 900_000, 0).await;
        assign(&app, &pro, false).await;
        // Hạ gói: hẹn tới hết kỳ.
        let body = json!({"plan_id": basic.0, "reference": "down-1"});
        let sub = ok(&app, Method::POST, "/plans/subscription/change", "owner", Some(body)).await;
        assert_eq!(
            (sub["data"]["plan_id"].clone(), sub["data"]["scheduled_plan_id"].clone()),
            (json!(pro.0), json!(basic.0))
        );
        // Nâng gói: ngay, kèm kỳ tỉ lệ (chênh 600 000, gần trọn kỳ còn lại).
        let body = json!({"plan_id": max.0, "price_id": max.1, "reference": "up-1"});
        let sub = ok(&app, Method::POST, "/plans/subscription/change", "owner", Some(body)).await;
        assert_eq!(
            (sub["data"]["plan_id"].clone(), sub["data"]["scheduled_plan_id"].clone()),
            (json!(max.0), Value::Null)
        );
        let periods = ok(&app, Method::GET, "/plans/billing-periods", "owner", None).await;
        let proration = periods["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["kind"] == "proration")
            .unwrap()
            .clone();
        let amount: i64 = proration["amount"]["amount"].as_str().unwrap().parse().unwrap();
        assert!((590_000..=600_000).contains(&amount), "{amount}");
        // Hạ gói lần nữa, rồi hết kỳ (gói rẻ không giá: không phải trả): áp gói mới.
        let body = json!({"plan_id": basic.0, "reference": "down-2"});
        ok(&app, Method::POST, "/plans/subscription/change", "owner", Some(body)).await;
        let id = proration["id"].as_str().unwrap();
        ok(&app, Method::POST, &format!("/plans/billing-periods/{id}/void"), "ops", None).await;
        sql(&app, &format!("UPDATE plans_subscription SET current_period_end = {PAST}")).await;
        tick(&app).await;
        let sub = ok(&app, Method::GET, "/plans/subscription", "owner", None).await;
        assert_eq!(
            (sub["data"]["plan_id"].clone(), sub["data"]["status"].clone()),
            (json!(basic.0), json!("active"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mail_nhac_khong_gui_trung() {
    TestDatabase::run(async |test| {
        let mails = Arc::new(Outbox::default());
        let app = app(test, &mails).await;
        let pro = priced(&app, "pro", 199_000, 0).await;
        assign(&app, &pro, false).await;
        let body = json!({"plan_id": pro.0, "price_id": pro.1, "billing_email": "billing@acme.example", "reference": "email"});
        ok(&app, Method::POST, "/plans/subscription/change", "owner", Some(body)).await;
        // Kỳ hết sau 12 giờ: job lập kỳ kế và nhắc "còn 1 ngày" đúng một lần.
        sql(&app, "UPDATE plans_subscription SET current_period_end = CURRENT_TIMESTAMP").await;
        let soon = chrono::Utc::now() + chrono::Duration::hours(12);
        sql(&app, &format!("UPDATE plans_subscription SET current_period_end = '{}'", soon.to_rfc3339())).await;
        tick(&app).await;
        tick(&app).await;
        let sent = mails.0.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0].to, "billing@acme.example");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chuyen_trang_thai_va_nhan_tien_dong_thoi() {
    TestDatabase::run(async |test| {
        let mails = Arc::new(Outbox::default());
        let app = Arc::new(app(test, &mails).await);
        let pro = priced(&app, "pro", 199_000, 0).await;
        assign(&app, &pro, false).await;
        sql(&app, &format!("UPDATE plans_subscription SET current_period_end = {PAST}")).await;
        tick(&app).await;
        assert_eq!(status(&app).await, "past_due");
        let periods = ok(&app, Method::GET, "/plans/billing-periods", "owner", None).await;
        let id = periods["data"][0]["id"].as_str().unwrap().to_owned();
        // Hết ân hạn đúng lúc nhận tiền: job và lần thu tiền chạy cùng lúc.
        sql(&app, &format!("UPDATE plans_subscription SET grace_ends_at = {PAST}")).await;
        let pay = {
            let app = app.clone();
            tokio::spawn(async move {
                let body = json!({"provider_reference": "VCB-9"});
                call(
                    &app,
                    Method::POST,
                    &format!("/plans/billing-periods/{id}/mark-paid"),
                    "ops",
                    Some(body),
                )
                .await
            })
        };
        let job = {
            let app = app.clone();
            tokio::spawn(async move { tick(&app).await })
        };
        let (paid, ran) = tokio::join!(pay, job);
        assert!(paid.unwrap().0.is_success());
        ran.unwrap();
        // Dù job chạy trước hay sau: kết quả là đang dùng (đã trả), không mất kỳ.
        assert_eq!(status(&app).await, "active");
    })
    .await
    .unwrap();
}
