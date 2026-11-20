//! Báo yêu thích có hàng lại / giảm giá (D-233): tự bật kèm đồng ý, chỉ giảm thật so với mốc, bỏ người
//! đã đăng ký ở `inventory`, gom lô tối đa 10, trần mỗi ngày, chỉ trong khung giờ, rút đồng ý thì dừng.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{BUYER, ConsentWithdrawn, Shop, call, drain, shop};

const ON: &str = "I want to hear when my wishlist products are back in stock or cheaper.";

async fn turn_on(shop: &Shop, who: &str, back_in_stock: bool, price_drop: bool) {
    let body = json!({
        "back_in_stock": back_in_stock, "price_drop": price_drop,
        "consent_text": ON, "consent_version": "2026-11",
    });
    let (status, answer) = call(shop, "PUT", "/me/wishlist/alerts", who, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

async fn like(shop: &Shop, who: &str, product: Uuid, variant: Option<Uuid>) {
    let body = json!({"product_id": product, "variant_id": variant});
    let (status, answer) = call(shop, "POST", "/me/wishlist", who, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

/// Tin báo yêu thích đã gửi.
fn alerts(shop: &Shop) -> Vec<Value> {
    shop.notifier
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|sent| sent.kind == "customers.wishlist_alert")
        .map(|sent| json!({"to": format!("{:?}", sent.recipient), "params": sent.params}))
        .collect()
}

async fn reprice(shop: &Shop, product: Uuid, variant: Uuid, price: i64) {
    shop.catalog.set_price(product, variant, price);
    shop.price_changed(product).await;
    drain(shop).await;
}

/// Chạy job gom lô trong khung giờ (rồi đóng lại).
async fn send(shop: &Shop) {
    shop.rules.open();
    shop.run_job(crate::jobs::SEND_WISHLIST_ALERTS).await;
    shop.rules.close();
}

#[tokio::test]
async fn bat_bao_can_dong_y_mac_dinh_tat() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (status, state) = call(&shop, "GET", "/me/wishlist/alerts", "buyer", None).await;
        assert_eq!(status, StatusCode::OK, "{state}");
        assert_eq!(
            state["data"],
            json!({"back_in_stock": false, "price_drop": false, "back_in_stock_available": true})
        );

        let body = json!({"back_in_stock": false, "price_drop": true});
        let (status, answer) = call(&shop, "PUT", "/me/wishlist/alerts", "buyer", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        assert_eq!(answer["code"], "customers.wishlist_alert_consent_required");

        turn_on(&shop, "buyer", true, true).await;
        assert_eq!(*shop.consents.1.lock().unwrap(), vec![Uuid::from_u128(BUYER)]);
        let body = json!({"back_in_stock": false, "price_drop": false});
        let (status, answer) = call(&shop, "PUT", "/me/wishlist/alerts", "buyer", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["data"]["price_drop"], false);
        // Tắt cả hai: rút đồng ý.
        assert!(shop.consents.1.lock().unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chi_bao_khi_giam_that_so_voi_moc() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        turn_on(&shop, "buyer", false, true).await;
        let (product, variant) = shop.catalog.product(100_000);
        like(&shop, "buyer", product, None).await;

        // Giảm 3% (< 5%): không báo.
        reprice(&shop, product, variant, 97_000).await;
        send(&shop).await;
        assert!(alerts(&shop).is_empty());
        // Tăng rồi về lại mốc: không phải giảm giá.
        reprice(&shop, product, variant, 120_000).await;
        reprice(&shop, product, variant, 100_000).await;
        send(&shop).await;
        assert!(alerts(&shop).is_empty());
        // Giảm 6%: báo một lần, giá báo là giá hiện tại.
        reprice(&shop, product, variant, 94_000).await;
        send(&shop).await;
        let sent = alerts(&shop);
        assert_eq!(sent.len(), 1, "{sent:?}");
        let item = &sent[0]["params"]["items"][0];
        assert_eq!(
            (item["kind"].as_str(), item["price"]["amount"].as_str()),
            (Some("price_drop"), Some("94000"))
        );
        // Event đến lại / giảm tiếp trong 7 ngày: không báo nữa.
        reprice(&shop, product, variant, 80_000).await;
        send(&shop).await;
        assert_eq!(alerts(&shop).len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn co_hang_lai_bo_qua_nguoi_da_dang_ky_o_inventory() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (product, variant) = shop.catalog.product(50_000);
        for who in ["buyer", "buyer2", "buyer3"] {
            turn_on(&shop, who, true, false).await;
        }
        like(&shop, "buyer", product, Some(variant)).await;
        // Theo sản phẩm: cũng được báo khi một biến thể có hàng lại.
        like(&shop, "buyer2", product, None).await;
        like(&shop, "buyer3", product, Some(variant)).await;
        // `buyer3` đã tự đăng ký ở `inventory`: tin do `inventory` gửi, không gửi trùng.
        shop.inventory.own_alerts.lock().unwrap().push((Uuid::from_u128(103), variant));

        shop.stock_changed(variant, 0, 5).await;
        drain(&shop).await;
        send(&shop).await;
        let sent = alerts(&shop);
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert!(sent.iter().all(|alert| {
            !alert["to"].as_str().unwrap().contains(&Uuid::from_u128(103).to_string())
        }));
        // Hết rồi có lại trong 7 ngày: không báo lần hai.
        shop.stock_changed(variant, 5, 0).await;
        shop.stock_changed(variant, 0, 3).await;
        drain(&shop).await;
        send(&shop).await;
        assert_eq!(alerts(&shop).len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gom_lo_toi_da_10_va_tran_moi_ngay() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        turn_on(&shop, "buyer", false, true).await;
        let mut products = Vec::new();
        for _ in 0..12 {
            let (product, variant) = shop.catalog.product(100_000);
            like(&shop, "buyer", product, None).await;
            products.push((product, variant));
        }
        for (product, variant) in &products {
            reprice(&shop, *product, *variant, 50_000).await;
        }
        send(&shop).await;
        let sent = alerts(&shop);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["params"]["count"], 10);
        // Trần mặc định một tin mỗi ngày: hai sản phẩm còn lại chờ hôm sau.
        send(&shop).await;
        assert_eq!(alerts(&shop).len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ngoai_khung_gio_thi_cho_lo_sau() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        turn_on(&shop, "buyer", false, true).await;
        let (product, variant) = shop.catalog.product(100_000);
        like(&shop, "buyer", product, None).await;
        reprice(&shop, product, variant, 80_000).await;
        // Ngoài khung giờ: không gửi, hàng đợi chờ.
        shop.run_job(crate::jobs::SEND_WISHLIST_ALERTS).await;
        assert!(alerts(&shop).is_empty());
        send(&shop).await;
        assert_eq!(alerts(&shop).len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn rut_dong_y_thi_tat_va_xoa_hang_doi() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        turn_on(&shop, "buyer", false, true).await;
        let (product, variant) = shop.catalog.product(100_000);
        like(&shop, "buyer", product, None).await;
        reprice(&shop, product, variant, 80_000).await;
        // Rút ở chỗ khác (link huỷ nhận của `notifications`): sổ đồng ý đổi rồi phát event.
        shop.consents.1.lock().unwrap().clear();
        shop.compliance_event(&ConsentWithdrawn(json!({
            "consent_id": Uuid::now_v7(), "purpose": crate::wishlist_alerts::PURPOSE,
            "scope": "", "channel": "",
        })))
        .await;
        drain(&shop).await;
        let (_, state) = call(&shop, "GET", "/me/wishlist/alerts", "buyer", None).await;
        assert_eq!(state["data"]["price_drop"], false, "{state}");
        send(&shop).await;
        assert!(alerts(&shop).is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_luot_gom_dong_thoi_chi_gui_mot_tin() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        turn_on(&shop, "buyer", false, true).await;
        let (product, variant) = shop.catalog.product(100_000);
        like(&shop, "buyer", product, None).await;
        reprice(&shop, product, variant, 80_000).await;
        shop.rules.open();
        // Hai job cùng chờ; worker chạy song song (4 luồng).
        let queue = autoxide_kernel::job::JobQueue::new("customers");
        for _ in 0..2 {
            queue
                .enqueue_database(
                    shop.database().write(),
                    &super::WishlistAlertsNow {},
                    autoxide_kernel::job::EnqueueOptions::default(),
                )
                .await
                .unwrap();
        }
        drain(&shop).await;
        shop.rules.close();
        let sent = alerts(&shop);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["params"]["count"], 1);
    })
    .await
    .unwrap();
}
