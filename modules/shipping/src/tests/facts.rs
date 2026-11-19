//! Trả lời tự động về hành trình vận đơn cho `support` (bước 44 mục 8, D-230): chỉ chủ đơn (tài
//! khoản người mua hay số điện thoại đã xác thực trùng số người nhận) nhận được câu trả lời; không
//! thấy, nhiều vận đơn, người lạ → `None`. Câu trả lời là mẫu câu i18n. Cả SQLite và Postgres.

use autoxide_support::api::{FactSource, Requester};

use super::rates::shop;
use super::*;
use crate::facts::{ShippingFacts, TRACKING, phrases};

fn vnd(amount: i64) -> Value {
    json!({"amount": amount.to_string(), "currency": "VND"})
}

fn shipment(tree: &super::rates::Tree, method: &str, order_number: &str, key: &str) -> Value {
    json!({
        "order_id": Uuid::now_v7(), "order_number": order_number, "request_key": key,
        "org_id": tree.a.as_uuid(), "method_id": method, "buyer_id": Uuid::from_u128(BUYER_X),
        "recipient_name": "Nguyen Van An", "recipient_phone": "0912 345 678",
        "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Trang Tien"},
        "items": [{"order_line_key": "l1", "variant_id": Uuid::now_v7(), "sku": "A", "name": "Ao", "quantity": 1, "weight_grams": 300}],
        "declared_value": vnd(100_000),
    })
}

#[test]
fn cum_tu_trang_thai_co_ban_dich() {
    let vi: Value = serde_json::from_str(include_str!("../../locales/vi.json")).unwrap();
    for phrase in phrases() {
        assert!(vi.get(format!("text:{phrase}")).is_some(), "missing translation of `{phrase}`");
    }
}

#[tokio::test]
async fn chi_chu_don_nhan_cau_tra_loi() {
    TestDatabase::run(async |test| {
        let (app, tree) = shop(test).await;
        let method = super::rates::method(&app, "boss", tree.chain, "flat", json!({})).await;
        let (status, created) =
            call(&app, "POST", "/shipments", "boss", Some(shipment(&tree, &method, "DH0042", "a"))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let facts = ShippingFacts::new(app.modules().translations().clone());
        assert_eq!(facts.intents()[0].intent, TRACKING);
        let tenant = app.default_tenant();
        let ask = |requester: Requester, params: Value, locale: &'static str| {
            let facts = facts.clone();
            let tenant = tenant.clone();
            async move { facts.answer(&tenant, &requester, TRACKING, &params, locale).await.unwrap() }
        };
        let order = json!({"order_number": "DH0042"});
        let buyer = Requester::User(Uuid::from_u128(BUYER_X));
        let answer = ask(buyer.clone(), order.clone(), "en").await.unwrap();
        assert!(answer.contains("DH0042") && answer.contains("being prepared"), "{answer}");
        let answer = ask(buyer.clone(), order.clone(), "vi-VN").await.unwrap();
        assert!(answer.contains("DH0042") && !answer.contains("being prepared"), "{answer}");
        // Số điện thoại đã xác thực, dạng E.164, trùng số người nhận.
        let phone = Requester::VerifiedPhone("+84912345678".to_owned());
        assert!(ask(phone, order.clone(), "en").await.is_some());
        // Người lạ, số khác, email, đơn không có, không có tham số, ý định khác → để nhân viên.
        assert_eq!(ask(Requester::User(Uuid::from_u128(BUYER_Y)), order.clone(), "en").await, None);
        assert_eq!(ask(Requester::VerifiedPhone("+84900000000".to_owned()), order.clone(), "en").await, None);
        assert_eq!(ask(Requester::VerifiedEmail("an@example.com".to_owned()), order.clone(), "en").await, None);
        assert_eq!(ask(buyer.clone(), json!({"order_number": "DH9999"}), "en").await, None);
        assert_eq!(ask(buyer.clone(), json!({}), "en").await, None);
        let other = facts.answer(&tenant, &buyer, "shipping.other", &order, "en").await.unwrap();
        assert_eq!(other, None);
        // Giao một phần (hai vận đơn cùng đơn): không chắc vận đơn nào → để nhân viên.
        let (status, _) =
            call(&app, "POST", "/shipments", "boss", Some(shipment(&tree, &method, "DH0042", "b"))).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(ask(buyer, order, "en").await, None);
    })
    .await
    .unwrap();
}
