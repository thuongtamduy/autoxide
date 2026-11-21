//! Khoá người mua cho hạn mức khuyến mãi và cờ `first_order` (đặc tả 43 "Chống lạm dụng", 45 "Giỏ:
//! coupon, voucher"): vãng lai và người đăng nhập đều có khoá số điện thoại + địa chỉ giao.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::json;

use super::checkout::{body, checkout, ready_cart};
use super::{Call, Shop, shop};

/// Khoá có tiền tố `prefix` trong giỏ giữ lượt gần nhất.
fn keys_of(shop: &Shop, prefix: &str) -> Vec<String> {
    let reserved = shop.promotions.reserved.lock().unwrap();
    let cart = reserved.last().expect("không có lượt giữ khuyến mãi");
    cart.buyer_keys.iter().filter(|key| key.starts_with(prefix)).cloned().collect()
}

fn last_first_order(shop: &Shop) -> bool {
    shop.promotions.last_evaluated.lock().unwrap().as_ref().expect("chưa báo giá").first_order
}

/// Đặt giỏ có mã giảm, trả (khoá user, khoá phone, khoá address) của lượt giữ.
async fn place_with_code(
    shop: &Shop,
    call: &Call<'_>,
    key: &str,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let shirt = shop.catalog.add(shop.tree.a, 200_000);
    shop.inventory.stock(shop.tree.a, shirt, 5);
    let (cart, token, _) = ready_cart(call, shop.tree.a, shirt, 1).await;
    let call = Call { app: call.app, who: call.who, cart_token: token.as_deref() };
    let (status, priced) =
        call.send("PUT", &format!("/carts/{cart}/codes"), Some(json!({"codes": ["GIAM10"]}))).await;
    assert_eq!(status, StatusCode::OK, "{priced}");
    let quote = priced["data"]["quote_token"].as_str().unwrap().to_owned();
    let (status, placed) = checkout(&call, Some(key), body(&cart, &quote, "cod")).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    (keys_of(shop, "user:"), keys_of(shop, "phone:"), keys_of(shop, "address:"))
}

#[tokio::test]
async fn vang_lai_va_dang_nhap_deu_co_khoa_so_dien_thoai_va_dia_chi() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        shop.promotions.add_code("GIAM10", 1000);
        let (user, phone, address) =
            place_with_code(&shop, &Call::as_guest(&shop, None), "guest-1").await;
        assert!(user.is_empty());
        assert_eq!(phone.len(), 1, "vãng lai phải có khoá số điện thoại");
        assert_eq!(address.len(), 1, "vãng lai phải có khoá địa chỉ giao");
        // Khoá là HMAC: không chứa số điện thoại thô.
        assert!(!phone[0].contains("0901234567") && !phone[0].contains("84901234567"));

        let (signed_user, signed_phone, signed_address) =
            place_with_code(&shop, &Call::as_user(&shop, "buyer1"), "user-1").await;
        assert_eq!(signed_user.len(), 1);
        // Cùng số, cùng địa chỉ: cùng khoá — đổi giữa vãng lai và đăng nhập vẫn bị đếm chung.
        assert_eq!(signed_phone, phone);
        assert_eq!(signed_address, address);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn first_order_theo_tai_khoan_va_so_dien_thoai() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 200_000);
        shop.inventory.stock(shop.tree.a, shirt, 50);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        assert!(last_first_order(&shop), "người mua chưa có đơn nào là đơn đầu");
        let (status, placed) = checkout(&buyer, Some("first"), body(&cart, &quote, "cod")).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        // Đã có đơn: giỏ mới không còn là đơn đầu.
        ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        assert!(!last_first_order(&shop));

        // Vãng lai cùng số nhận hàng với đơn trước: không phải đơn đầu.
        ready_cart(&Call::as_guest(&shop, None), shop.tree.a, shirt, 1).await;
        assert!(!last_first_order(&shop));

        // Vãng lai số khác: đơn đầu; vãng lai chưa có số: không biết là ai → không phải đơn đầu.
        for (phone, expected) in [(Some("0912345678"), true), (None, false)] {
            let guest = Call::as_guest(&shop, None);
            let (_, created) =
                guest.send("POST", "/carts", Some(json!({"org_id": shop.tree.a.as_uuid()}))).await;
            let id = created["data"]["id"].as_str().unwrap().to_owned();
            let token = created["data"]["cart_token"].as_str().unwrap().to_owned();
            let guest = Call::as_guest(&shop, Some(&token));
            guest
                .send("PUT", &format!("/carts/{id}/lines/{shirt}"), Some(json!({"quantity": 1})))
                .await;
            let mut delivery = json!({"address": {"country": "VN", "province_code": "01",
                "commune_code": "00004", "line": "9 Hang Bai"}, "recipient_name": "Binh",
                "shipping_option": "standard"});
            if let Some(phone) = phone {
                delivery["phone"] = json!(phone);
            }
            let (status, answer) =
                guest.send("PUT", &format!("/carts/{id}/delivery"), Some(delivery)).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            assert_eq!(last_first_order(&shop), expected, "phone {phone:?}");
        }
    })
    .await
    .unwrap();
}
