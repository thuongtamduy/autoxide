//! Số câu SQL của giỏ hàng và đặt hàng (D-20261120-step39-hot-paths): không tăng theo số dòng của
//! giỏ (không N+1). Module khác là bộ giả; chỉ đếm câu chạm bảng `orders_` — câu của kernel
//! (cache cấu hình, khoá gốc) chạy hay không tuỳ cache còn hạn, làm số lệch ±1 giữa các lượt.

use super::CatalogExt as _;
use autoxide_testkit::TestDatabase;
use serde_json::json;

use super::checkout::body;
use super::{Call, shop};

/// Số câu chạm bảng của `orders`.
fn own(statements: &[String]) -> usize {
    statements.iter().filter(|sql| sql.contains("\"orders_")).count()
}

/// Đếm câu SQL của từng bước mua với giỏ có `lines` dòng.
async fn measure(test: &TestDatabase, lines: usize) -> Vec<(&'static str, usize)> {
    let shop = shop(test).await;
    let app = &shop.app;
    let buyer = Call::as_user(app, "buyer1");
    let mut variants = Vec::new();
    for _ in 0..lines {
        let variant = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, variant, 50);
        variants.push(variant);
    }
    let (_, created) =
        buyer.send("POST", "/carts", Some(json!({"org_id": shop.tree.a.as_uuid()}))).await;
    let cart = created["data"]["id"].as_str().unwrap().to_owned();
    let (last, rest) = variants.split_last().unwrap();
    for variant in rest {
        let (status, answer) = buyer
            .send("PUT", &format!("/carts/{cart}/lines/{variant}"), Some(json!({"quantity": 1})))
            .await;
        assert!(status.is_success(), "{answer}");
    }
    let counted = std::cell::RefCell::new(Vec::new());
    let step = async |name: &'static str, method: &str, uri: String, input| {
        let ((status, answer), statements) = app.statements(buyer.send(method, &uri, input)).await;
        assert!(status.is_success(), "{name}: {answer}");
        counted.borrow_mut().push((name, own(&statements)));
        answer
    };
    step("PUT line", "PUT", format!("/carts/{cart}/lines/{last}"), Some(json!({"quantity": 1})))
        .await;
    step("GET cart", "GET", format!("/carts/{cart}"), None).await;
    let delivery = json!({"address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1 Trang Tien"},
        "recipient_name": "An", "phone": "0901234567", "shipping_option": "standard"});
    let priced =
        step("PUT delivery", "PUT", format!("/carts/{cart}/delivery"), Some(delivery)).await;
    let quote = priced["data"]["quote_token"].as_str().unwrap().to_owned();
    let request = app
        .post("/api/v1/orders/checkout")
        .token("buyer1")
        .header("Idempotency-Key", "k-1")
        .json(body(&cart, &quote, "cod"));
    let (placed, statements) = app.statements(request.send()).await;
    let placed = placed.unwrap();
    assert!(placed.status().is_success(), "{}", placed.text());
    counted.borrow_mut().push(("POST checkout", own(&statements)));
    let order = placed.json()["data"]["orders"][0]["id"].as_str().unwrap().to_owned();
    step("GET me/order", "GET", format!("/me/orders/{order}"), None).await;
    step("GET me/orders", "GET", "/me/orders".to_owned(), None).await;
    counted.into_inner()
}

/// Hai dòng hay bảy dòng trong giỏ: cùng số câu SQL.
#[tokio::test]
async fn khong_tang_theo_so_dong() {
    // Từ hai dòng: dòng đầu vào giỏ trống chạy thêm một câu, không phải N+1.
    let one = TestDatabase::run(async |test| measure(test, 2).await).await.unwrap();
    let many = TestDatabase::run(async |test| measure(test, 7).await).await.unwrap();
    eprintln!("orders SQL: 2 = {one:?}\n7 = {many:?}");
    assert_eq!(one, many);
}
