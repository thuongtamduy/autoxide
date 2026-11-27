//! Đặt hàng trên chương trình thật (cả SQLite và Postgres, bước 45): sản phẩm và giá của `catalog`
//! thật, giỏ vãng lai, đặt hàng trả qua `ledger` thật với cổng thanh toán giả (webhook ký đúng; có
//! cả MoMo nên đơn đi qua cổng mặc định của khách) → đơn `confirmed`; gửi lại cùng `Idempotency-Key` trả đơn cũ.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::PaymentGateway;
use autoxide_testkit::{FakePaymentGateway, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1{uri}"));
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..10 {
        let events = dispatcher.run_once().await.unwrap();
        if worker.run_once().await.unwrap() + events == 0 {
            break;
        }
    }
}

const BOSS: [(&str, &str); 1] = [("authorization", "Bearer boss")];

/// Chương trình thật với cổng giả làm cổng mặc định, và một sản phẩm số giá 150.000 đang bán: (app, cổng,
/// biến thể).
async fn shop(test: &TestDatabase) -> (TestApp, Arc<FakePaymentGateway>, String) {
    let gateway = Arc::new(FakePaymentGateway::new());
    let mut builder = TestApp::builder(test)
        .principal("boss", Principal::with_all_permissions(Uuid::from_u128(1)))
        .contribute::<dyn PaymentGateway>(gateway.clone());
    // Bài này thu qua cổng giả; bộ nối thật `momo` cũng góp `dyn PaymentGateway`. Có hai cổng mà đơn
    // không nêu đối tác: khách chọn cổng mặc định (bước 70 P8).
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    let app = builder.build().await.unwrap();
    let boss = BOSS;
    let setting = "/kernel/settings/ledger.payments.default_provider";
    let (status, set) = call(&app, "PUT", setting, &boss, Some(json!({"value": "fake"}))).await;
    assert_eq!(status, StatusCode::OK, "{set}");

    // Sản phẩm số (không giao, không tồn kho) đang bán, giá 150.000 gồm thuế.
    let product = json!({"kind": "digital", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Khoá học"}, "variant": {"sku": "COURSE-1"}});
    let (status, created) = call(&app, "POST", "/catalog/products", &boss, Some(product)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let product_id = created["data"]["id"].as_str().unwrap().to_owned();
    let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
    let (status, list) = call(
        &app,
        "POST",
        "/catalog/price-lists",
        &boss,
        Some(json!({"name": "Bán lẻ", "currency": "VND", "tax_inclusive": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{list}");
    let prices = json!({"reference": "opening", "prices": [{"variant_id": variant, "amount": {"amount": "150000", "currency": "VND"}}]});
    let uri = format!("/catalog/price-lists/{}/prices", list["data"]["id"].as_str().unwrap());
    let (status, added) = call(&app, "POST", &uri, &boss, Some(prices)).await;
    assert_eq!(status, StatusCode::OK, "{added}");
    // Hàng số cần một file trước khi mở bán: tải lên qua `files`, gắn vào biến thể.
    let uploaded = app
        .post("/api/v1/files?name=course.pdf")
        .header("authorization", "Bearer boss")
        .header("content-type", "application/pdf")
        .bytes(b"%PDF-1.4 course".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(uploaded.status(), StatusCode::CREATED, "{}", uploaded.text());
    let file = json!({"file_id": uploaded.json()["data"]["id"]});
    let uri = format!("/catalog/variants/{variant}/digital-assets");
    let (status, attached) = call(&app, "POST", &uri, &boss, Some(file)).await;
    assert_eq!(status, StatusCode::CREATED, "{attached}");
    let uri = format!("/catalog/products/{product_id}/status");
    let (status, active) =
        call(&app, "POST", &uri, &[boss[0], ("if-match", "*")], Some(json!({"status": "active"})))
            .await;
    assert_eq!(status, StatusCode::OK, "{active}");

    (app, gateway, variant)
}

/// Giỏ vãng lai 2 sản phẩm (300.000): (giỏ, `Cart-Token`, `quote_token`).
async fn cart(app: &TestApp, variant: &str) -> (String, String, String) {
    // Giỏ vãng lai, giá do máy chủ tính.
    let (status, cart) = call(app, "POST", "/orders/carts", &[], Some(json!({}))).await;
    assert_eq!(status, StatusCode::CREATED, "{cart}");
    let cart_id = cart["data"]["id"].as_str().unwrap().to_owned();
    let token = cart["data"]["cart_token"].as_str().unwrap().to_owned();
    let guest = [("cart-token", token.as_str())];
    let (status, cart) = call(
        app,
        "PUT",
        &format!("/orders/carts/{cart_id}/lines/{variant}"),
        &guest,
        Some(json!({"quantity": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cart}");
    assert_eq!(cart["data"]["totals"]["total"]["amount"], "300000", "{cart}");
    assert_eq!(cart["data"]["shipping"]["required"], false);
    let quote = cart["data"]["quote_token"].as_str().unwrap().to_owned();

    (cart_id, token, quote)
}

/// Đặt hàng trả bằng thẻ qua cổng mặc định với khoá `key`: (id đơn, id lượt thu ở `ledger`).
async fn place(
    app: &TestApp,
    gateway: &FakePaymentGateway,
    variant: &str,
    key: &str,
) -> (String, Uuid) {
    let (cart_id, token, quote) = cart(app, variant).await;
    let body = json!({"cart_id": cart_id, "quote_token": quote, "payment_method": "card",
        "contact": {"name": "An", "phone": "0901234567"}});
    let headers = [("cart-token", token.as_str()), ("idempotency-key", key)];
    let (status, placed) = call(app, "POST", "/orders/checkout", &headers, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    let order = placed["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
    let ledger_payment: Uuid = placed["data"]["payment_actions"][0]["ledger_payment_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(gateway.payments.lock().unwrap().contains_key(&ledger_payment));
    (order, ledger_payment)
}

/// Đối tác báo đã thu `amount` cho lượt thu `payment` (webhook ký đúng), rồi chạy hết event và job.
async fn collected(
    app: &TestApp,
    gateway: &FakePaymentGateway,
    payment: Uuid,
    amount: i64,
    event: &str,
) {
    let (headers, body) = gateway.paid(payment, Money::new(amount, Currency::VND), event);
    let mut request = app.request(Method::POST, "/api/v1/ledger/webhooks/fake").bytes(body);
    request = request.header("content-type", "application/json");
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    assert!(request.send().await.unwrap().status().is_success());
    drain(app).await;
}

/// Đặt cột thời gian `column` của dòng `id` ở bảng `table` về một phút trước.
async fn past(app: &TestApp, table: &str, column: &str, id: &str) {
    use sea_orm::ConnectionTrait as _;
    let write = app.database().begin_write().await.unwrap();
    let backend = write.get_database_backend();
    let sql = format!("UPDATE {table} SET {column} = $1 WHERE id = $2");
    let at = autoxide_kernel::time::Timestamp::now() - jiff::SignedDuration::from_mins(1);
    let id: Uuid = id.parse().unwrap();
    write
        .execute_raw(autoxide_kernel::database::statement(backend, sql, [at.into(), id.into()]))
        .await
        .unwrap();
    write.commit().await.unwrap();
}

/// Các mục ở hàng chờ đối soát có trạng thái `status`.
async fn exceptions(app: &TestApp, status: &str) -> Vec<Value> {
    let uri = format!("/ledger/exceptions?status={status}");
    let (code, listed) = call(app, "GET", &uri, &BOSS, None).await;
    assert_eq!(code, StatusCode::OK, "{listed}");
    listed["data"].as_array().unwrap().clone()
}

async fn order_state(app: &TestApp, order: &str) -> Value {
    let (status, got) = call(app, "GET", &format!("/orders/{order}"), &BOSS, None).await;
    assert_eq!(status, StatusCode::OK, "{got}");
    got["data"].clone()
}

#[tokio::test]
async fn gio_vang_lai_dat_hang_tra_qua_cong_thi_don_xac_nhan() {
    TestDatabase::run(async |test| {
        let (app, gateway, variant) = shop(test).await;
        let boss = BOSS;
        let (cart_id, token, quote) = cart(&app, &variant).await;
        let guest = [("cart-token", token.as_str())];

        // Đặt hàng, trả bằng thẻ qua cổng mặc định của khách.
        let body = json!({"cart_id": cart_id, "quote_token": quote, "payment_method": "card",
            "contact": {"name": "An", "phone": "0901234567"}});
        let headers = [guest[0], ("idempotency-key", "e2e-1")];
        let (status, placed) =
            call(&app, "POST", "/orders/checkout", &headers, Some(body.clone())).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        let order = &placed["data"]["orders"][0];
        assert_eq!(order["status"], "pending_payment");
        assert_eq!(placed["data"]["payment_actions"][0]["kind"], "redirect", "{placed}");
        let order_id = order["id"].as_str().unwrap().to_owned();
        let (status, again) = call(&app, "POST", "/orders/checkout", &headers, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["orders"][0]["id"].as_str(), Some(order_id.as_str()));

        // Đối tác báo đã thu → `ledger.payment_succeeded` → đơn `confirmed`.
        let payment = *gateway.payments.lock().unwrap().keys().next().unwrap();
        let (headers, body) =
            gateway.paid(payment, Money::new(300_000, Currency::VND), "evt-order-1");
        let mut request = app.request(Method::POST, "/api/v1/ledger/webhooks/fake").bytes(body);
        request = request.header("content-type", "application/json");
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        assert!(request.send().await.unwrap().status().is_success());
        drain(&app).await;
        let (status, got) = call(&app, "GET", &format!("/orders/{order_id}"), &boss, None).await;
        assert_eq!(status, StatusCode::OK, "{got}");
        assert_eq!(
            (got["data"]["status"].as_str(), got["data"]["payment_status"].as_str()),
            (Some("confirmed"), Some("paid"))
        );
        assert_eq!(got["data"]["totals"]["paid"]["amount"], "300000");
    })
    .await
    .unwrap();
}

/// Rà #591 C1 — một luật "muộn": tiền về sau hạn của lượt thu ở `ledger` nhưng trước khi job huỷ đơn chạy.
/// `ledger` mở `late_payment`; `orders` (bên quyết) tính tiền vào đơn và đơn đi tiếp, nên mục tự đóng
/// (`kept`) — không còn mục bảo người vận hành hoàn tay tiền của đơn đang giao.
#[tokio::test]
async fn tien_ve_sau_han_truoc_khi_huy_don_thi_don_di_tiep_va_muc_dong() {
    TestDatabase::run(async |test| {
        let (app, gateway, variant) = shop(test).await;
        let (order, payment) = place(&app, &gateway, &variant, "late-1").await;
        past(&app, "ledger_payments", "expires_at", &payment.to_string()).await;
        collected(&app, &gateway, payment, 300_000, "evt-late-1").await;

        let got = order_state(&app, &order).await;
        assert_eq!(
            (got["status"].as_str(), got["payment_status"].as_str()),
            (Some("confirmed"), Some("paid")),
            "{got}"
        );
        let open = exceptions(&app, "open").await;
        assert!(open.iter().all(|item| item["kind"] != "late_payment"), "{open:?}");
        let resolved = exceptions(&app, "resolved").await;
        let item = resolved
            .iter()
            .find(|item| item["kind"] == "late_payment")
            .unwrap_or_else(|| panic!("{resolved:?}"));
        assert_eq!(item["subject"], format!("ledger.payment:{payment}"));
        assert_eq!(item["resolution"], "kept");
        assert!(gateway.refunds.lock().unwrap().is_empty(), "nothing is refunded");
    })
    .await
    .unwrap();
}

#[derive(Debug, serde::Serialize)]
struct ExpireUnpaidNow {}

impl autoxide_kernel::job::Job for ExpireUnpaidNow {
    const KIND: &'static str = "orders.expire_unpaid";
}

/// Rà #591 V1: đối tác báo sai số tiền (`needs_review`), người đối soát chỉ gỡ sau hạn của đơn. Lúc đó đơn
/// đã bị huỷ vì quá hạn, nên `accept_received` ghi khoản đã thu rồi tiền được hoàn tự động qua đối tác — đơn
/// không mở lại; mục `late_payment` tự đóng khi đã hoàn đủ.
#[tokio::test]
async fn go_needs_review_sau_han_thi_don_da_huy_va_tien_tu_hoan() {
    TestDatabase::run(async |test| {
        let (app, gateway, variant) = shop(test).await;
        let (order, payment) = place(&app, &gateway, &variant, "review-1").await;
        collected(&app, &gateway, payment, 250_000, "evt-short-1").await;
        let (_, got) = call(&app, "GET", &format!("/ledger/payments/{payment}"), &BOSS, None).await;
        assert_eq!(got["data"]["status"], "needs_review", "{got}");

        // Hạn của đơn (và của lượt thu, cùng một hạn) trôi qua; job huỷ đơn quá hạn chạy.
        past(&app, "ledger_payments", "expires_at", &payment.to_string()).await;
        past(&app, "orders_orders", "payment_due_at", &order).await;
        autoxide_kernel::job::JobQueue::new("orders")
            .enqueue_database(
                app.database().write(),
                &ExpireUnpaidNow {},
                autoxide_kernel::job::EnqueueOptions::default(),
            )
            .await
            .unwrap();
        drain(&app).await;
        assert_eq!(order_state(&app, &order).await["status"], "cancelled");

        let open = exceptions(&app, "open").await;
        let item = open
            .iter()
            .find(|item| item["kind"] == "amount_mismatch")
            .unwrap_or_else(|| panic!("{open:?}"));
        let uri = format!("/ledger/exceptions/{}/resolve", item["id"].as_str().unwrap());
        let body = json!({"action": "accept_received", "payment_id": payment});
        let (status, resolved) = call(&app, "POST", &uri, &BOSS, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{resolved}");
        drain(&app).await;

        let got = order_state(&app, &order).await;
        assert_eq!(got["status"], "cancelled", "the order is not reopened: {got}");
        let refunds = gateway.refunds.lock().unwrap().clone();
        assert_eq!(refunds.len(), 1, "{refunds:?}");
        assert_eq!(refunds[0].amount, Money::new(250_000, Currency::VND));
        let open = exceptions(&app, "open").await;
        assert!(open.is_empty(), "{open:?}");
    })
    .await
    .unwrap();
}
