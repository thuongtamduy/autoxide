//! Hoàn tất thanh toán giả trong sandbox (UT-4, D-20261127-sandbox-payment-simulation):
//! `POST /api/v1/ledger/payments/{id}/simulate` đi đúng đường của webhook (thu / hỏng / lệch số), chỉ
//! khi người vận hành bật, chỉ trong sandbox, chỉ cho lượt thu do bộ giả của sandbox tạo, chỉ khi còn
//! `pending`; gọi lại cùng `Idempotency-Key` không ghi trùng. Chạy trên cả SQLite và Postgres.

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::payment::PaymentGateway;
use autoxide_kernel::tenancy::{TenantContext, TenantId, root_of};
use autoxide_kernel::time::Timestamp;
use autoxide_ledger::api::{Ledger, NewPayment, Payment};
use autoxide_testkit::{FakePaymentGateway, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::ExprTrait as _;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const PASSWORD: &str = "correct horse battery staple";
const EMAIL: &str = "owner@shop.example";

async fn app(test: &TestDatabase, enabled: bool) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-sandbox-pay-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .tenant("shop");
    if enabled {
        builder = builder.setting("AUTOXIDE_LEDGER_SANDBOX_SIMULATION", "true");
    }
    builder
        .module(autoxide_mail::module())
        .module(autoxide_auth::module())
        // Kho `Idempotency-Key` (lệnh ghi chạm tiền đi qua lớp idempotency của kernel).
        .module(autoxide_idempotency::module())
        .contribute::<dyn PaymentGateway>(Arc::new(FakePaymentGateway::new()))
        .module(autoxide_ledger::module())
        .build()
        .await
        .unwrap()
}

/// Một request tới `/api/v1…` ở khách `tenant` (slug hay ID sandbox); trả (status, header
/// `Idempotent-Replayed`, thân).
async fn call(
    app: &TestApp,
    method: Method,
    path: &str,
    token: &str,
    tenant: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Option<String>, Value) {
    let mut request =
        app.request(method, &format!("/api/v1{path}")).header("autoxide-tenant", tenant);
    if !token.is_empty() {
        request = request.token(token);
    }
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.unwrap();
    let replayed = response
        .headers()
        .get("idempotent-replayed")
        .map(|value| value.to_str().unwrap().to_owned());
    let body = if response.body().is_empty() { Value::Null } else { response.json() };
    (response.status(), replayed, body)
}

/// Chủ khách `shop` (admin, email đã xác minh), đã đăng nhập; trả access token.
async fn owner(app: &TestApp, shop: &TenantContext) -> String {
    let account = NewAccount {
        email: EMAIL.to_owned(),
        name: "Owner".to_owned(),
        password: Some(PASSWORD.to_owned()),
        admin: true,
    };
    app.api::<dyn Accounts>().unwrap().create(shop, account).await.unwrap();
    let verified = sea_orm::sea_query::Query::update()
        .table(sea_orm::sea_query::Alias::new("auth_user"))
        .value(
            sea_orm::sea_query::Alias::new("email_verified_at"),
            sea_orm::Value::from(Timestamp::now()),
        )
        .and_where(sea_orm::sea_query::Expr::col(sea_orm::sea_query::Alias::new("email")).eq(EMAIL))
        .to_owned();
    shop.database().write().execute(&verified).await.unwrap();
    let body = json!({"email": EMAIL, "password": PASSWORD});
    let (status, _, answer) =
        call(app, Method::POST, "/auth/login", "", "shop", None, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["data"]["access_token"].as_str().unwrap().to_owned()
}

async fn settle(app: &TestApp) {
    let worker = app.modules().worker(8).unwrap().blocked_delay(Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..5 {
        let ran = worker.run_once().await.unwrap();
        let delivered = dispatcher.run_once().await.unwrap();
        if ran == 0 && delivered == 0 {
            break;
        }
    }
}

/// Sandbox của `shop`, đã sẵn sàng; trả (ID, khách).
async fn sandbox(app: &TestApp, token: &str, shop: &TenantContext) -> (String, TenantContext) {
    let path = format!("/kernel/tenants/{}/sandboxes", shop.id());
    let (status, _, created) =
        call(app, Method::POST, &path, token, "shop", None, Some(json!({"name": "Try"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    settle(app).await;
    let tenant = app.registry().tenant(TenantId::from_uuid(id.parse().unwrap())).await.unwrap();
    assert!(tenant.is_sandbox());
    (id, tenant)
}

/// Lượt thu 150.000 VND ở khách `tenant`, mã tham chiếu `reference`.
async fn pay(app: &TestApp, tenant: &TenantContext, reference: &str) -> Payment {
    let amount = Money::from_columns(150_000, "VND").unwrap();
    let payment =
        NewPayment::new(root_of(tenant.id()), "card", amount, "Order 1001", "demo", reference);
    let ledger = app.api::<dyn Ledger>().unwrap();
    ledger.create_payment(tenant, Actor::system(), payment).await.unwrap().into_inner()
}

fn simulate_path(payment: &Payment) -> String {
    format!("/ledger/payments/{}/simulate", payment.id)
}

#[tokio::test]
async fn gia_lap_mac_dinh_tat() {
    TestDatabase::run(async |test| {
        let app = app(test, false).await;
        let shop = app.tenant("shop").await.unwrap();
        let token = owner(&app, &shop).await;
        let (id, sandbox) = sandbox(&app, &token, &shop).await;
        let payment = pay(&app, &sandbox, "S-1").await;
        let body = Some(json!({"outcome": "succeeded"}));
        let (status, _, refused) =
            call(&app, Method::POST, &simulate_path(&payment), &token, &id, None, body).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("ledger.simulation_disabled")),
            "{refused}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gia_lap_chi_trong_sandbox_va_di_duong_cua_webhook() {
    TestDatabase::run(async |test| {
        let app = app(test, true).await;
        let shop = app.tenant("shop").await.unwrap();
        let token = owner(&app, &shop).await;
        let (id, sandbox) = sandbox(&app, &token, &shop).await;
        let succeeded = Some(json!({"outcome": "succeeded"}));

        // Khách thật: từ chối, kể cả với lượt thu thật.
        let real = pay(&app, &shop, "R-1").await;
        let (status, _, refused) = call(
            &app,
            Method::POST,
            &simulate_path(&real),
            &token,
            "shop",
            None,
            succeeded.clone(),
        )
        .await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::CONFLICT, Some("tenancy.sandbox_only")),
            "{refused}"
        );

        // Sandbox, đúng số: đã thu; gọi lại cùng khoá trả bản lưu, không ghi gì thêm.
        let payment = pay(&app, &sandbox, "S-1").await;
        assert!(payment.provider_ref.as_deref().unwrap().starts_with("SANDBOX-"));
        let path = simulate_path(&payment);
        let (status, replayed, paid) =
            call(&app, Method::POST, &path, &token, &id, Some("sim-1"), succeeded.clone()).await;
        assert_eq!(status, StatusCode::OK, "{paid}");
        assert_eq!(replayed, None);
        assert_eq!(paid["data"]["status"], "succeeded", "{paid}");
        let (status, replayed, again) =
            call(&app, Method::POST, &path, &token, &id, Some("sim-1"), succeeded.clone()).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(replayed.as_deref(), Some("true"));
        assert_eq!(again, paid);
        // Khoá khác: lượt thu không còn `pending`.
        let (status, _, refused) =
            call(&app, Method::POST, &path, &token, &id, Some("sim-2"), succeeded.clone()).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::CONFLICT, Some("ledger.payment_state")),
            "{refused}"
        );
        settle(&app).await;
        let ledger = app.api::<dyn Ledger>().unwrap();
        let stored = ledger.payment(&sandbox, payment.id).await.unwrap().unwrap();
        assert_eq!(stored.status, "succeeded");

        // Người mua không trả: `failed`.
        let other = pay(&app, &sandbox, "S-2").await;
        let failed = Some(json!({"outcome": "failed"}));
        let (status, _, answer) =
            call(&app, Method::POST, &simulate_path(&other), &token, &id, None, failed).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["data"]["status"], "failed");

        // Số khác số phải thu: `needs_review`, như thông báo thật.
        let short = pay(&app, &sandbox, "S-3").await;
        let body = Some(json!({"outcome": "succeeded",
            "amount": {"amount": "100000", "currency": "VND"}}));
        let (status, _, answer) =
            call(&app, Method::POST, &simulate_path(&short), &token, &id, None, body).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["data"]["status"], "needs_review");

        // Lượt thu không do bộ giả của sandbox tạo (cờ `sandbox` tắt, hay mã không phải `SANDBOX-`): từ
        // chối — tiền tố một mình không đủ, cờ một mình không đủ.
        for (flag, reference) in [(false, None), (true, Some("real-partner-123"))] {
            let row = pay(&app, &sandbox, &format!("S-flag-{flag}")).await;
            let mut update = sea_orm::sea_query::Query::update();
            update
                .table(sea_orm::sea_query::Alias::new("ledger_payments"))
                .value(sea_orm::sea_query::Alias::new("sandbox"), flag)
                .and_where(
                    sea_orm::sea_query::Expr::col(sea_orm::sea_query::Alias::new("id")).eq(row.id),
                );
            if let Some(reference) = reference {
                update.value(sea_orm::sea_query::Alias::new("provider_ref"), reference);
            }
            sandbox.database().write().execute(&update.to_owned()).await.unwrap();
            let (status, _, refused) = call(
                &app,
                Method::POST,
                &simulate_path(&row),
                &token,
                &id,
                None,
                succeeded.clone(),
            )
            .await;
            assert_eq!(
                (status, refused["code"].as_str()),
                (StatusCode::CONFLICT, Some("ledger.payment_not_simulated")),
                "{refused}"
            );
        }
    })
    .await
    .unwrap();
}
