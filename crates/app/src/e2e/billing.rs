//! Tính tiền thuê bao (bước 35 mục 4) trên chương trình thật: kỳ mở → hoá đơn do khách vận hành lập
//! (`invoices`, số không nhảy, ghi sổ) → khách trả qua cổng thanh toán của khách vận hành (`ledger`)
//! → đối tác báo đã thu → kỳ đã trả, thuê bao đang dùng lại. Chạy trên cả SQLite và Postgres.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::PaymentGateway;
use autoxide_testkit::{FakePaymentGateway, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

const OPS: [&str; 8] = [
    "plans.plans.manage",
    "plans.subscriptions.manage",
    "plans.subscriptions.read",
    "invoices.read",
    "invoices.series_manage",
    "tax.packs_manage",
    "ledger.read",
    "kernel.tenants.read",
];

async fn app(test: &TestDatabase, gateway: Arc<FakePaymentGateway>) -> TestApp {
    // Thư và file ghi vào thư mục tạm, không vào `crates/app/data/` của repo.
    let dir = std::env::temp_dir().join(format!("autoxide-billing-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .contribute::<dyn PaymentGateway>(gateway)
        .tenant("acme")
        // Đối tác gọi webhook qua tên miền của khách vận hành (khách suy từ tên miền).
        .domain("default", "pay.operator.example")
        .user("ops", 1, &OPS)
        .user_in("acme", "owner", 2, &["plans.billing.read", "plans.billing.manage"]);
    // Bài này thu qua cổng giả; bộ nối thật `momo` cũng góp `dyn PaymentGateway`, mà có hai cổng thì
    // mỗi thanh toán phải nêu đối tác, nên không nạp nó ở đây.
    for module in crate::modules::all().into_iter().filter(|module| module.name() != "momo") {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
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

async fn sql(app: &TestApp, text: &str) {
    use sea_orm::{ConnectionTrait, Statement};
    let control = app.registry().control().write();
    control
        .execute_raw(Statement::from_string(control.get_database_backend(), text))
        .await
        .unwrap();
}

/// Chạy job và phát event tới khi hết việc.
async fn settle(app: &TestApp) {
    sql(app, "DELETE FROM plans_jobs").await;
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..6 {
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

async fn open_period(app: &TestApp) -> Value {
    let periods = ok(app, Method::GET, "/plans/billing-periods", "owner", None).await;
    periods["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|period| period["status"] == "open")
        .cloned()
        .unwrap()
}

#[tokio::test]
async fn ky_thanh_toan_thanh_hoa_don_tra_qua_cong_thi_da_tra() {
    TestDatabase::run(async |test| {
        let gateway = Arc::new(FakePaymentGateway::new());
        let app = app(test, gateway.clone()).await;
        // Khách vận hành: gói thuế, ký hiệu hoá đơn.
        let pack: Value = serde_json::from_str(include_str!("../../../../modules/tax/packs/vn.json")).unwrap();
        ok(&app, Method::POST, "/tax/packs", "ops", Some(json!({"pack": pack}))).await;
        let year = Timestamp::now().date_utc().year();
        ok(&app, Method::POST, "/invoices/series", "ops", Some(json!({"kind": "vat_invoice", "symbol": "1C26SAS", "year": year}))).await;
        // Gói 199.000 đ / tháng, khách `acme` đang dùng; hết kỳ → kỳ gia hạn mở.
        let plan = ok(
            &app,
            Method::POST,
            "/plans/plans",
            "ops",
            Some(json!({"code": "pro", "name": "Pro", "status": "active", "grace_days": 7, "suspend_days": 30, "retention_days": 90})),
        )
        .await;
        let plan_id = plan["data"]["id"].as_str().unwrap().to_owned();
        let price = json!({"kind": "recurring", "interval": "month", "amount": {"amount": "199000", "currency": "VND"}});
        let plan = ok(&app, Method::POST, &format!("/plans/plans/{plan_id}/prices"), "ops", Some(price)).await;
        let price_id = plan["data"]["prices"][0]["id"].as_str().unwrap().to_owned();
        let acme = app.tenant("acme").await.unwrap().id();
        ok(
            &app,
            Method::PUT,
            &format!("/plans/tenants/{acme}/subscription"),
            "ops",
            Some(json!({"plan_id": plan_id, "price_id": price_id, "trial": false})),
        )
        .await;
        sql(&app, "UPDATE plans_subscription SET current_period_end = '2000-01-01T00:00:00Z'").await;
        settle(&app).await;
        let period = open_period(&app).await;
        let id = period["id"].as_str().unwrap().to_owned();
        // Hoá đơn của khách vận hành cho kỳ: số 1 của ký hiệu, tổng = giá gói (đã gồm thuế).
        assert_eq!(period["invoice_number"], "1C26SAS-0000001", "{period}");
        let invoice = period["invoice_id"].as_str().unwrap();
        let document = ok(&app, Method::GET, &format!("/invoices/documents/{invoice}"), "ops", None).await;
        assert_eq!(document["data"]["total"]["amount"], "199000");
        assert_eq!(document["data"]["source_module"], "plans");
        // Trả: cần hồ sơ thanh toán.
        let (status, refused) = call(&app, Method::POST, &format!("/plans/billing-periods/{id}/pay"), "owner", Some(json!({}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["code"], "plans.billing_profile_required");
        let profile = json!({"legal_name": "Acme Co", "tax_code": "0101234567", "email": "billing@acme.example"});
        ok(&app, Method::PUT, "/plans/billing-profile", "owner", Some(profile)).await;
        let paying = ok(&app, Method::POST, &format!("/plans/billing-periods/{id}/pay"), "owner", Some(json!({}))).await;
        assert!(paying["data"]["checkout_url"].as_str().is_some(), "{paying}");
        // Gọi lại: cùng lượt thu.
        let again = ok(&app, Method::POST, &format!("/plans/billing-periods/{id}/pay"), "owner", Some(json!({}))).await;
        assert_eq!(again["data"]["payment_id"], paying["data"]["payment_id"]);
        // Đối tác báo đã thu → kỳ đã trả, thuê bao đang dùng lại.
        let payment = paying["data"]["payment_id"].as_str().unwrap().parse().unwrap();
        let (headers, body) = gateway.paid(payment, Money::new(199_000, Currency::VND), "evt-1");
        let mut request =
            app.request(Method::POST, "/api/v1/ledger/webhooks/fake").host("pay.operator.example").bytes(body);
        request = request.header("content-type", "application/json");
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        assert_eq!(request.send().await.unwrap().status(), StatusCode::OK);
        settle(&app).await;
        let paid = ok(&app, Method::GET, &format!("/plans/billing-periods/{id}"), "owner", None).await;
        assert_eq!(paid["data"]["status"], "paid", "{paid}");
        let sub = ok(&app, Method::GET, "/plans/subscription", "owner", None).await;
        assert_eq!(sub["data"]["status"], "active");
        // Sổ của khách vận hành: phải thu khách hàng về 0 (hoá đơn ghi nợ, tiền về ghi có).
        let books = ok(&app, Method::GET, "/ledger/books", "ops", None).await;
        let book = books["data"][0]["id"].as_str().unwrap();
        let accounts = ok(&app, Method::GET, &format!("/ledger/accounts?book_id={book}&purpose=receivable.customer"), "ops", None).await;
        assert_eq!(accounts["data"][0]["balance"]["amount"], "0", "{accounts}");
        // Trả kỳ đã trả: 409.
        let (status, _) = call(&app, Method::POST, &format!("/plans/billing-periods/{id}/pay"), "owner", Some(json!({}))).await;
        assert_eq!(status, StatusCode::CONFLICT);
    })
    .await
    .unwrap();
}
