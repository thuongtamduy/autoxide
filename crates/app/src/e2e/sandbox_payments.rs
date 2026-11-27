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
    account(app, shop, EMAIL, true).await
}

/// Tài khoản `email` của khách `shop` (`admin`: role quản trị; không thì không có permission nào), email đã
/// xác minh, đã đăng nhập; trả access token.
async fn account(app: &TestApp, shop: &TenantContext, email: &str, admin: bool) -> String {
    let account = NewAccount {
        email: email.to_owned(),
        name: "Staff".to_owned(),
        password: Some(PASSWORD.to_owned()),
        admin,
    };
    app.api::<dyn Accounts>().unwrap().create(shop, account).await.unwrap();
    let verified = sea_orm::sea_query::Query::update()
        .table(sea_orm::sea_query::Alias::new("auth_user"))
        .value(
            sea_orm::sea_query::Alias::new("email_verified_at"),
            sea_orm::Value::from(Timestamp::now()),
        )
        .and_where(sea_orm::sea_query::Expr::col(sea_orm::sea_query::Alias::new("email")).eq(email))
        .to_owned();
    shop.database().write().execute(&verified).await.unwrap();
    let body = json!({"email": email, "password": PASSWORD});
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

/// Số dòng của bảng `table` trong database của `tenant` có `column = value`.
async fn count(tenant: &TenantContext, table: &str, column: &str, value: &str) -> i64 {
    let database = tenant.database();
    let backend = database.read().get_database_backend();
    let sql = format!("SELECT COUNT(*) AS n FROM {table} WHERE {column} = '{value}'");
    let row = database
        .read()
        .query_one_raw(autoxide_kernel::database::statement(backend, sql, []))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "n").unwrap()
}

/// Số event `ledger.payment_succeeded` của lượt thu `payment` trong database của `tenant`.
async fn succeeded_events(tenant: &TenantContext, payment: uuid::Uuid) -> usize {
    let database = tenant.database();
    let backend = database.read().get_database_backend();
    let sql = "SELECT payload FROM ledger_events WHERE name = 'ledger.payment_succeeded'";
    let rows = database
        .read()
        .query_all_raw(autoxide_kernel::database::statement(backend, sql, []))
        .await
        .unwrap();
    rows.iter()
        .map(|row| row.try_get::<String>("", "payload").unwrap())
        .filter(|payload| {
            serde_json::from_str::<Value>(payload).unwrap()["payment_id"] == json!(payment)
        })
        .count()
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
            call(&app, Method::POST, &simulate_path(&payment), &token, &id, None, body.clone())
                .await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("ledger.simulation_disabled")),
            "{refused}"
        );
        // Thiếu `ledger.reconcile`: 403, không phải 404 — người không có quyền không biết tính năng bật hay tắt.
        let clerk = account(&app, &shop, "clerk@shop.example", false).await;
        let (status, _, refused) =
            call(&app, Method::POST, &simulate_path(&payment), &clerk, "shop", None, body).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied")),
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

        // Thiếu `ledger.reconcile`: 403 trước mọi kiểm khác (không lộ tính năng bật hay tắt).
        let clerk = account(&app, &shop, "clerk@shop.example", false).await;
        let (status, _, refused) = call(
            &app,
            Method::POST,
            &simulate_path(&real),
            &clerk,
            "shop",
            None,
            succeeded.clone(),
        )
        .await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied")),
            "{refused}"
        );
        // ID lượt thu của khách thật gọi trong sandbox: không có ở đó.
        let (status, _, missing) =
            call(&app, Method::POST, &simulate_path(&real), &token, &id, None, succeeded.clone())
                .await;
        assert_eq!(
            (status, missing["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("ledger.payment_not_found")),
            "{missing}"
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
        // Cùng đường của webhook: bút toán ghi thu và `ledger.payment_succeeded` cho module nguồn, đúng một.
        let reference = format!("payment:{}:captured", payment.id);
        assert_eq!(count(&sandbox, "ledger_entries", "reference", &reference).await, 1);
        assert_eq!(succeeded_events(&sandbox, payment.id).await, 1);

        // Qua tool MCP (agent): cùng service, cùng kết quả.
        let by_agent = pay(&app, &sandbox, "S-mcp").await;
        let mcp = app.mcp(&token).await.unwrap().with_header("autoxide-tenant", &id);
        let answer = mcp
            .call_tool(autoxide_testkit::tool(
                "ledger.simulate_payment",
                json!({"payment_id": by_agent.id, "outcome": "succeeded"}),
            ))
            .await
            .unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        let stored = ledger.payment(&sandbox, by_agent.id).await.unwrap().unwrap();
        assert_eq!(stored.status, "succeeded");

        // Đầu vào sai: mọi lỗi báo một lần (D-083), lượt thu vẫn `pending`.
        let invalid = pay(&app, &sandbox, "S-invalid").await;
        let body =
            Some(json!({"outcome": "paid", "amount": {"amount": "150000", "currency": "USD"}}));
        let (status, _, refused) =
            call(&app, Method::POST, &simulate_path(&invalid), &token, &id, None, body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        let mut fields: Vec<&str> = refused["errors"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|error| error["field"].as_str())
            .collect();
        fields.sort_unstable();
        assert_eq!(fields, ["amount", "outcome"], "{refused}");
        let stored = ledger.payment(&sandbox, invalid.id).await.unwrap().unwrap();
        assert_eq!(stored.status, "pending");

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

/// Hai lệnh giả lập cùng lúc cho một lượt thu (một "đã thu", một "hỏng", khoá khác nhau): đúng một thông báo được
/// lưu, lượt thu có đúng một kết quả, không bao giờ hai bút toán hay hai event `ledger.payment_succeeded`.
#[tokio::test]
async fn hai_lenh_gia_lap_dong_thoi_chi_mot_ket_qua() {
    TestDatabase::run(async |test| {
        let app = app(test, true).await;
        let shop = app.tenant("shop").await.unwrap();
        let token = owner(&app, &shop).await;
        let (id, sandbox) = sandbox(&app, &token, &shop).await;
        let payment = pay(&app, &sandbox, "S-race").await;
        let path = simulate_path(&payment);
        let succeeded = call(
            &app,
            Method::POST,
            &path,
            &token,
            &id,
            Some("race-1"),
            Some(json!({"outcome": "succeeded"})),
        );
        let failed = call(
            &app,
            Method::POST,
            &path,
            &token,
            &id,
            Some("race-2"),
            Some(json!({"outcome": "failed"})),
        );
        let ((first, _, one), (second, _, two)) = tokio::join!(succeeded, failed);
        // Mỗi lệnh: 200 (đã trả lời, hay thấy lượt thu như đang có) hoặc 409 (lượt thu đã hết `pending`).
        for (status, answer) in [(first, &one), (second, &two)] {
            assert!(
                status == StatusCode::OK
                    || status == StatusCode::CONFLICT && answer["code"] == "ledger.payment_state",
                "{status} {answer}"
            );
        }
        settle(&app).await;
        let notice = format!("sandbox:{}", payment.id);
        assert_eq!(count(&sandbox, "ledger_gateway_events", "provider_event_id", &notice).await, 1);
        let ledger = app.api::<dyn Ledger>().unwrap();
        let stored = ledger.payment(&sandbox, payment.id).await.unwrap().unwrap();
        let captured = format!("payment:{}:captured", payment.id);
        let (entries, events) = (
            count(&sandbox, "ledger_entries", "reference", &captured).await,
            succeeded_events(&sandbox, payment.id).await,
        );
        match stored.status.as_str() {
            "succeeded" => assert_eq!((entries, events), (1, 1)),
            "failed" => assert_eq!((entries, events), (0, 0)),
            other => panic!("unexpected status {other}"),
        }
    })
    .await
    .unwrap();
}

/// Như hướng dẫn hứa: trong sandbox, đặt một đơn trả bằng thẻ (cổng giả), tìm lượt thu của đơn theo
/// `source_id`, giả lập "đã thu" → đơn `confirmed` / `paid`. Đủ mọi module như chương trình thật.
#[tokio::test]
async fn don_trong_sandbox_tra_bang_gia_lap_thi_xac_nhan() {
    TestDatabase::run(async |test| {
        let dir =
            std::env::temp_dir().join(format!("autoxide-sandbox-order-{}", uuid::Uuid::now_v7()));
        let mut builder = TestApp::builder(test)
            .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
            .setting("AUTOXIDE_LEDGER_SANDBOX_SIMULATION", "true")
            .tenant("shop")
            .contribute::<dyn PaymentGateway>(Arc::new(FakePaymentGateway::new()));
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let shop = app.tenant("shop").await.unwrap();
        let token = owner(&app, &shop).await;
        let (id, _) = sandbox(&app, &token, &shop).await;
        let staff = |method: Method, path: &str, body: Option<Value>| {
            let (app, token, id) = (&app, token.clone(), id.clone());
            let path = path.to_owned();
            async move { call(app, method, &path, &token, &id, None, body).await }
        };

        // Cửa hàng trong sandbox: cổng mặc định, một sản phẩm số đang bán giá 150.000.
        let (status, _, set) = staff(
            Method::PUT,
            "/kernel/settings/ledger.payments.default_provider",
            Some(json!({"value": "fake"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{set}");
        let product = json!({"kind": "digital", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Khoá học"}, "variant": {"sku": "COURSE-1"}});
        let (status, _, created) = staff(Method::POST, "/catalog/products", Some(product)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let product_id = created["data"]["id"].as_str().unwrap().to_owned();
        let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
        let list = json!({"name": "Bán lẻ", "currency": "VND", "tax_inclusive": true});
        let (status, _, list) = staff(Method::POST, "/catalog/price-lists", Some(list)).await;
        assert_eq!(status, StatusCode::CREATED, "{list}");
        let prices = json!({"reference": "opening", "prices": [{"variant_id": variant,
            "amount": {"amount": "150000", "currency": "VND"}}]});
        let uri = format!("/catalog/price-lists/{}/prices", list["data"]["id"].as_str().unwrap());
        let (status, _, added) = staff(Method::POST, &uri, Some(prices)).await;
        assert_eq!(status, StatusCode::OK, "{added}");
        let uploaded = app
            .post("/api/v1/files?name=course.pdf")
            .token(&token)
            .header("autoxide-tenant", &id)
            .header("content-type", "application/pdf")
            .bytes(b"%PDF-1.4 course".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(uploaded.status(), StatusCode::CREATED, "{}", uploaded.text());
        let file = json!({"file_id": uploaded.json()["data"]["id"]});
        let uri = format!("/catalog/variants/{variant}/digital-assets");
        let (status, _, attached) = staff(Method::POST, &uri, Some(file)).await;
        assert_eq!(status, StatusCode::CREATED, "{attached}");
        let active = app
            .post(&format!("/api/v1/catalog/products/{product_id}/status"))
            .token(&token)
            .header("autoxide-tenant", &id)
            .header("if-match", "*")
            .json(json!({"status": "active"}))
            .send()
            .await
            .unwrap();
        assert_eq!(active.status(), StatusCode::OK, "{}", active.text());

        // Người mua vãng lai trong sandbox: giỏ, đặt hàng trả bằng thẻ.
        let guest = |method: Method, path: &str, cart: Option<&str>, body: Value| {
            let mut request =
                app.request(method, &format!("/api/v1{path}")).header("autoxide-tenant", &id);
            if let Some(cart) = cart {
                request = request.header("cart-token", cart);
            }
            request.json(body)
        };
        let cart = guest(Method::POST, "/orders/carts", None, json!({})).send().await.unwrap();
        assert_eq!(cart.status(), StatusCode::CREATED, "{}", cart.text());
        let cart = cart.json();
        let cart_id = cart["data"]["id"].as_str().unwrap().to_owned();
        let cart_token = cart["data"]["cart_token"].as_str().unwrap().to_owned();
        let uri = format!("/orders/carts/{cart_id}/lines/{variant}");
        let line = guest(Method::PUT, &uri, Some(&cart_token), json!({"quantity": 1}));
        let line = line.send().await.unwrap();
        assert_eq!(line.status(), StatusCode::OK, "{}", line.text());
        let quote = line.json()["data"]["quote_token"].as_str().unwrap().to_owned();
        let body = json!({"cart_id": cart_id, "quote_token": quote, "payment_method": "card",
            "contact": {"name": "An", "phone": "0901234567"}});
        let placed = guest(Method::POST, "/orders/checkout", Some(&cart_token), body)
            .header("idempotency-key", "sandbox-order-1")
            .send()
            .await
            .unwrap();
        assert_eq!(placed.status(), StatusCode::CREATED, "{}", placed.text());
        let order = placed.json()["data"]["orders"][0]["id"].as_str().unwrap().to_owned();

        // Lượt thu của đơn theo `source_id`, rồi đóng vai đối tác.
        let uri = format!("/ledger/payments?source_module=orders&source_id={order}");
        let (status, _, found) = staff(Method::GET, &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{found}");
        let payment = found["data"][0]["id"].as_str().unwrap().to_owned();
        let uri = format!("/ledger/payments/{payment}/simulate");
        let (status, _, paid) =
            staff(Method::POST, &uri, Some(json!({"outcome": "succeeded"}))).await;
        assert_eq!(status, StatusCode::OK, "{paid}");
        settle(&app).await;
        let (status, _, got) = staff(Method::GET, &format!("/orders/{order}"), None).await;
        assert_eq!(status, StatusCode::OK, "{got}");
        assert_eq!(got["data"]["status"], "confirmed", "{got}");
        assert_eq!(got["data"]["payment_status"], "paid");
    })
    .await
    .unwrap();
}
