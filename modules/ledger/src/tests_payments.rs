//! Test thanh toán của `ledger` (cả SQLite và Postgres): webhook sai chữ ký không lưu, webhook gửi
//! lại đồng thời chỉ xử lý một lần, đối tác không trả lời rồi gửi lại cùng `reference`, hoàn tiền
//! đồng thời không vượt số đã thu, đối soát báo thiếu và lệch, xử lý tay một mục chờ.

use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{PaymentGateway, StatementKind, StatementLine};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_testkit::{FakePaymentGateway, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{AccountKey, Counterparty, Ledger, NewPayment, NewRefund};

const ALL: [&str; 4] =
    ["ledger.read", "ledger.payments_create", "ledger.refunds_create", "ledger.reconcile"];

async fn app(test: &TestDatabase) -> (TestApp, Arc<FakePaymentGateway>) {
    let gateway = Arc::new(FakePaymentGateway::new());
    let app = TestApp::builder(test)
        .user("cashier", 1, &ALL)
        .contribute::<dyn PaymentGateway>(gateway.clone())
        .module(crate::module())
        .build()
        .await
        .unwrap();
    (app, gateway)
}

fn ledger(app: &TestApp) -> Arc<dyn Ledger> {
    app.api::<dyn Ledger>().unwrap()
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

async fn call(app: &TestApp, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/ledger{uri}")).token("cashier");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn webhook(app: &TestApp, signed: (Vec<(String, String)>, Vec<u8>)) -> StatusCode {
    let (headers, body) = signed;
    let mut request = app.request(Method::POST, "/api/v1/ledger/webhooks/fake").bytes(body);
    request = request.header("content-type", "application/json");
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    request.send().await.unwrap().status()
}

/// Chạy hết job đang chờ (xử lý thông báo, niêm phong…).
async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..5 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

async fn pay(app: &TestApp, amount: i64, reference: &str) -> crate::api::Payment {
    let tenant = app.default_tenant();
    let payment =
        NewPayment::new(root_of(DEFAULT_TENANT), "card", vnd(amount), "Order", "demo", reference);
    ledger(app).create_payment(&tenant, Actor::system(), payment).await.unwrap().into_inner()
}

async fn clearing(app: &TestApp) -> i64 {
    let key = AccountKey::new(root_of(DEFAULT_TENANT), "clearing.gateway", Currency::VND)
        .with_counterparty(Counterparty::new("partner", "fake"));
    ledger(app).balance(&app.default_tenant(), key, None).await.unwrap().minor()
}

async fn status(app: &TestApp, id: Uuid) -> String {
    ledger(app).payment(&app.default_tenant(), id).await.unwrap().unwrap().status
}

async fn exceptions(app: &TestApp) -> Vec<Value> {
    let (_, listed) = call(app, "GET", "/exceptions?status=open", None).await;
    listed["data"].as_array().unwrap().clone()
}

#[tokio::test]
async fn webhook_sai_chu_ky_tra_401_va_khong_luu() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let payment = pay(&app, 100_000, "P-1").await;
        let (headers, mut body) = gateway.paid(payment.id, vnd(100_000), "evt-1");
        body.push(b' ');
        assert_eq!(webhook(&app, (headers, body)).await, StatusCode::UNAUTHORIZED);
        // Chữ ký đúng nhưng quá cửa sổ thời gian: phát lại bị chặn.
        let stale = FakePaymentGateway::signed(
            &json!({"event_id": "evt-2", "kind": "payment_succeeded", "provider_ref": payment.provider_ref,
                    "amount": 100_000, "currency": "VND"}),
            Utc::now().timestamp() - 3600,
        );
        assert_eq!(webhook(&app, stale).await, StatusCode::UNAUTHORIZED);
        drain(&app).await;
        assert_eq!(status(&app, payment.id).await, "pending");
        assert_eq!(clearing(&app).await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn webhook_gui_lai_dong_thoi_chi_xu_ly_mot_lan() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let payment = pay(&app, 250_000, "P-1").await;
        assert!(payment.checkout_url.is_some());
        let signed = gateway.paid(payment.id, vnd(250_000), "evt-1");
        let statuses = futures_like_join(&app, signed, 8).await;
        assert!(statuses.iter().all(|status| *status == StatusCode::OK), "{statuses:?}");
        // Một thông báo khác (mã khác) cho cùng thanh toán cũng không ghi thêm.
        assert_eq!(
            webhook(&app, gateway.paid(payment.id, vnd(250_000), "evt-2")).await,
            StatusCode::OK
        );
        drain(&app).await;
        assert_eq!(status(&app, payment.id).await, "succeeded");
        assert_eq!(clearing(&app).await, 250_000);
        let (_, entries) = call(&app, "GET", "/entries?source_module=ledger", None).await;
        assert_eq!(entries["data"].as_array().unwrap().len(), 1, "{entries}");
        // Thông báo "thất bại" đến muộn không làm lùi trạng thái.
        let failed = gateway.webhook(&json!({"event_id": "evt-3", "kind": "payment_failed",
            "provider_ref": payment.provider_ref, "amount": 250_000, "currency": "VND"}));
        assert_eq!(webhook(&app, failed).await, StatusCode::OK);
        drain(&app).await;
        assert_eq!(status(&app, payment.id).await, "succeeded");
    })
    .await
    .unwrap();
}

/// Gửi cùng webhook `times` lần cùng lúc (các future chạy xen nhau, đủ để đụng nhau ở database).
async fn futures_like_join(
    app: &TestApp,
    signed: (Vec<(String, String)>, Vec<u8>),
    times: usize,
) -> Vec<StatusCode> {
    futures_util::future::join_all((0..times).map(|_| webhook(app, signed.clone()))).await
}

#[tokio::test]
async fn doi_tac_khong_tra_loi_roi_gui_lai_cung_reference() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        gateway.fail_next(1);
        let body = json!({"method": "card", "amount": {"amount": "50000", "currency": "VND"},
                          "description": "Order 7", "reference": "P-7", "source_id": "order-7"});
        let (status, answer) = call(&app, "POST", "/payments", Some(body.clone())).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
        assert_eq!(answer["code"], "payment.provider_unavailable");
        let (status, again) = call(&app, "POST", "/payments", Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert!(again["data"]["checkout_url"].as_str().is_some(), "{again}");
        let (status, third) = call(&app, "POST", "/payments", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(third["data"]["id"], again["data"]["id"]);
        assert_eq!(gateway.payments.lock().unwrap().len(), 1);
        let (_, listed) = call(&app, "GET", "/payments?source_id=order-7", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hoan_tien_dong_thoi_khong_vuot_so_da_thu() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let payment = pay(&app, 100_000, "P-1").await;
        assert_eq!(webhook(&app, gateway.paid(payment.id, vnd(100_000), "evt-1")).await, StatusCode::OK);
        drain(&app).await;
        let tenant = app.default_tenant();
        let tasks: Vec<_> = (0..12)
            .map(|index| {
                let (ledger, tenant) = (ledger(&app), tenant.clone());
                tokio::spawn(async move {
                    let refund = NewRefund::new(payment.id, vnd(30_000), "Returned", format!("R-{index}"));
                    ledger.refund(&tenant, Actor::system(), refund).await
                })
            })
            .collect();
        let mut ok = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => ok += 1,
                Err(error) => assert_eq!(error.code(), "ledger.refund_exceeds_payment"),
            }
        }
        assert_eq!(ok, 3);
        let found = ledger(&app).payment(&tenant, payment.id).await.unwrap().unwrap();
        assert_eq!(found.refunded.minor(), 90_000);
        assert_eq!(clearing(&app).await, 10_000);
        // Qua REST: vượt số còn lại → 422.
        let (status, answer) = call(
            &app,
            "POST",
            &format!("/payments/{}/refunds", payment.id),
            Some(json!({"amount": {"amount": "20000", "currency": "VND"}, "reason": "More", "reference": "R-X"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["code"], "ledger.refund_exceeds_payment");
        let (_, detail) = call(&app, "GET", &format!("/payments/{}", payment.id), None).await;
        assert_eq!(detail["data"]["refunds"].as_array().unwrap().len(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_soat_bao_thieu_va_lech_roi_xu_ly_tay() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let today = Utc::now().date_naive();
        // A: đã thu qua webhook, có trong sao kê (kèm phí). B: webhook lạc, sao kê có → ghi như đã thu.
        // C: đã thu nhưng sao kê không có. D: sao kê báo số khác. E: sao kê có giao dịch lạ.
        let a = pay(&app, 100_000, "A").await;
        let b = pay(&app, 200_000, "B").await;
        let c = pay(&app, 300_000, "C").await;
        let d = pay(&app, 400_000, "D").await;
        for (found, event) in [(&a, "evt-a"), (&c, "evt-c")] {
            let signed = gateway.paid(found.id, found.amount, event);
            assert_eq!(webhook(&app, signed).await, StatusCode::OK);
        }
        drain(&app).await;
        let line = |id: &str, found: &crate::api::Payment, amount: i64| {
            StatementLine::new(
                id,
                found.provider_ref.clone().unwrap(),
                StatementKind::Payment,
                vnd(amount),
                today,
            )
        };
        gateway.set_statement(vec![
            line("L-a", &a, 100_000).fee(vnd(1_100)),
            line("L-b", &b, 200_000),
            line("L-d", &d, 399_000),
            StatementLine::new("L-e", "fake_unknown", StatementKind::Payment, vnd(5_000), today),
        ]);
        let (status, run) =
            call(&app, "POST", "/reconciliations", Some(json!({"date": today}))).await;
        assert_eq!(status, StatusCode::CREATED, "{run}");
        assert_eq!(run["data"]["lines"], 4);
        assert_eq!(run["data"]["matched"], 2);
        assert_eq!(status_of(&app, b.id).await, "succeeded");
        assert_eq!(status_of(&app, d.id).await, "needs_review");
        let open = exceptions(&app).await;
        let kinds: Vec<&str> = open.iter().map(|item| item["kind"].as_str().unwrap()).collect();
        for kind in ["missing_in_statement", "amount_mismatch", "unknown_in_statement"] {
            assert!(kinds.contains(&kind), "{kinds:?}");
        }
        // Phí cổng ghi giảm `clearing.gateway`.
        assert_eq!(clearing(&app).await, 100_000 + 200_000 + 300_000 - 1_100);
        // Chạy lại cùng ngày: không thêm dòng, không mở trùng.
        let (status, _) =
            call(&app, "POST", "/reconciliations", Some(json!({"date": today}))).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(exceptions(&app).await.len(), open.len());
        assert_eq!(clearing(&app).await, 600_000 - 1_100);
        // Xử lý tay: chấp nhận số đối tác thực thu cho D.
        let mismatch = open.iter().find(|item| item["kind"] == "amount_mismatch").unwrap();
        let (status, resolved) = call(
            &app,
            "POST",
            &format!("/exceptions/{}/resolve", mismatch["id"].as_str().unwrap()),
            Some(json!({"action": "match", "payment_id": d.id, "note": "Partner short-paid"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{resolved}");
        assert_eq!(resolved["data"]["status"], "resolved");
        assert_eq!(status_of(&app, d.id).await, "succeeded");
        assert_eq!(clearing(&app).await, 600_000 - 1_100 + 399_000);
    })
    .await
    .unwrap();
}

async fn status_of(app: &TestApp, id: Uuid) -> String {
    status(app, id).await
}

#[tokio::test]
async fn so_tien_khac_va_tien_ve_tre_vao_hang_cho() {
    TestDatabase::run(async |test| {
        let (app, gateway) = app(test).await;
        let short = pay(&app, 100_000, "S").await;
        assert_eq!(
            webhook(&app, gateway.paid(short.id, vnd(90_000), "evt-s")).await,
            StatusCode::OK
        );
        drain(&app).await;
        assert_eq!(status(&app, short.id).await, "needs_review");
        assert_eq!(clearing(&app).await, 0);
        let kinds: Vec<String> = exceptions(&app)
            .await
            .iter()
            .map(|item| item["kind"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(kinds, ["amount_mismatch"]);
        // Không cấu hình cổng → lỗi rõ.
        let tenant = app.default_tenant();
        let other = NewPayment::new(root_of(DEFAULT_TENANT), "card", vnd(1), "X", "demo", "N")
            .provider("vnpay");
        let error = ledger(&app).create_payment(&tenant, Actor::system(), other).await.unwrap_err();
        assert_eq!(error.code(), "ledger.gateway_not_configured");
    })
    .await
    .unwrap();
}
