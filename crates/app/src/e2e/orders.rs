//! Đặt hàng trên chương trình thật (cả SQLite và Postgres, bước 45): sản phẩm và giá của `catalog`
//! thật, giỏ vãng lai, đặt hàng trả qua `ledger` thật với cổng thanh toán giả (webhook ký đúng) →
//! đơn `confirmed`; gửi lại cùng `Idempotency-Key` trả đơn cũ.

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

#[tokio::test]
async fn gio_vang_lai_dat_hang_tra_qua_cong_thi_don_xac_nhan() {
    TestDatabase::run(async |test| {
        let gateway = Arc::new(FakePaymentGateway::new());
        let mut builder = TestApp::builder(test)
            .principal("boss", Principal::with_all_permissions(Uuid::from_u128(1)))
            .contribute::<dyn PaymentGateway>(gateway.clone());
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let boss = [("authorization", "Bearer boss")];

        // Sản phẩm số (không giao, không tồn kho) đang bán, giá 150.000 gồm thuế.
        let product = json!({"kind": "digital", "tax_category": "vat.standard", "locale": "vi",
            "text": {"name": "Khoá học"}, "variant": {"sku": "COURSE-1"}});
        let (status, created) = call(&app, "POST", "/catalog/products", &boss, Some(product)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let product_id = created["data"]["id"].as_str().unwrap().to_owned();
        let variant = created["data"]["variants"][0]["id"].as_str().unwrap().to_owned();
        let (status, list) = call(&app, "POST", "/catalog/price-lists", &boss,
            Some(json!({"name": "Bán lẻ", "currency": "VND", "tax_inclusive": true}))).await;
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
        let (status, active) = call(&app, "POST", &uri, &[boss[0], ("if-match", "*")], Some(json!({"status": "active"}))).await;
        assert_eq!(status, StatusCode::OK, "{active}");

        // Giỏ vãng lai, giá do máy chủ tính.
        let (status, cart) = call(&app, "POST", "/orders/carts", &[], Some(json!({}))).await;
        assert_eq!(status, StatusCode::CREATED, "{cart}");
        let cart_id = cart["data"]["id"].as_str().unwrap().to_owned();
        let token = cart["data"]["cart_token"].as_str().unwrap().to_owned();
        let guest = [("cart-token", token.as_str())];
        let (status, cart) =
            call(&app, "PUT", &format!("/orders/carts/{cart_id}/lines/{variant}"), &guest, Some(json!({"quantity": 2}))).await;
        assert_eq!(status, StatusCode::OK, "{cart}");
        assert_eq!(cart["data"]["totals"]["total"]["amount"], "300000", "{cart}");
        assert_eq!(cart["data"]["shipping"]["required"], false);
        let quote = cart["data"]["quote_token"].as_str().unwrap().to_owned();

        // Đặt hàng, trả bằng thẻ qua cổng.
        let body = json!({"cart_id": cart_id, "quote_token": quote, "payment_method": "card",
            "contact": {"name": "An", "phone": "0901234567"}});
        let headers = [guest[0], ("idempotency-key", "e2e-1")];
        let (status, placed) = call(&app, "POST", "/orders/checkout", &headers, Some(body.clone())).await;
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
        let (headers, body) = gateway.paid(payment, Money::new(300_000, Currency::VND), "evt-order-1");
        let mut request = app.request(Method::POST, "/api/v1/ledger/webhooks/fake").bytes(body);
        request = request.header("content-type", "application/json");
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        assert!(request.send().await.unwrap().status().is_success());
        drain(&app).await;
        let (status, got) = call(&app, "GET", &format!("/orders/{order_id}"), &boss, None).await;
        assert_eq!(status, StatusCode::OK, "{got}");
        assert_eq!((got["data"]["status"].as_str(), got["data"]["payment_status"].as_str()), (Some("confirmed"), Some("paid")));
        assert_eq!(got["data"]["totals"]["paid"]["amount"], "300000");
    })
    .await
    .unwrap();
}
