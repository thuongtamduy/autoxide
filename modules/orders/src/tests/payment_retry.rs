//! Người mua tự mở lại lượt thu (bước 70 P8): đối tác từ chối lúc đặt → `payment_actions[].error`
//! nói lý do; người mua vãng lai (token xem đơn) hay đã đăng nhập mở lại bằng
//! `POST …/payment-link`, chống trùng theo `Idempotency-Key`, có giới hạn mỗi giờ.

use super::CatalogExt as _;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::checkout::{body, checkout, ready_cart};
use super::guests::{access_token, guests, public};
use super::{Call, Shop};

/// Đặt một đơn trả qua cổng (`card`) khi đối tác từ chối khởi tạo; trả (id, số đơn, câu trả lời).
async fn refused_order(shop: &Shop, who: Option<&str>) -> (String, String, Value) {
    let shirt = shop.catalog.add(shop.tree.a, 200_000);
    shop.inventory.stock(shop.tree.a, shirt, 5);
    let call = match who {
        Some(who) => Call::as_user(shop, who),
        None => Call::as_guest(shop, None),
    };
    let (cart, token, quote) = ready_cart(&call, shop.tree.a, shirt, 1).await;
    let call = Call { app: call.app, who: call.who, cart_token: token.as_deref() };
    shop.ledger.fail_next_payments(1);
    let key = uuid::Uuid::now_v7().to_string();
    let (status, placed) = checkout(&call, Some(&key), body(&cart, &quote, "card")).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    let order = &placed["data"]["orders"][0];
    let id = order["id"].as_str().unwrap().to_owned();
    let number = order["number"].as_str().unwrap().to_owned();
    (id, number, placed)
}

#[tokio::test]
async fn doi_tac_tu_choi_luc_dat_thi_bao_ly_do() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let (_, _, placed) = refused_order(&guests.shop, None).await;
        let order = &placed["data"]["orders"][0];
        assert_eq!(order["status"], "pending_payment");
        let action = &placed["data"]["payment_actions"][0];
        assert_eq!(action["kind"], "none", "{action}");
        assert_eq!(action["error"]["code"], "ledger.gateway_not_configured", "{action}");
        assert_eq!(
            action["error"]["message"],
            "The payment partner `fake` is not set up on this server."
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_mua_vang_lai_tu_mo_lai_luot_thu() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let shop = &guests.shop;
        let (order, number, _) = refused_order(shop, None).await;
        let token = access_token(&guests, &number).await;
        let uri = format!("/public/orders/{order}/payment-link");

        // Không token / token sai: 401.
        let (status, _) = public(shop, "POST", &uri, None, None, Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Đối tác vẫn từ chối: trả lỗi của đối tác.
        shop.ledger.fail_next_payments(1);
        let (status, refused) =
            public(shop, "POST", &uri, Some(&token), None, Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["code"], "ledger.gateway_not_configured", "{refused}");

        // Mở lại với `return_url`: có link, hết lý do lỗi; `return_url` tới đối tác.
        let retry = json!({"return_url": "https://shop.example/thanks"});
        let (status, opened) =
            public(shop, "POST", &uri, Some(&token), Some("r1"), Some(retry.clone())).await;
        assert_eq!(status, StatusCode::OK, "{opened}");
        let action = &opened["data"][0];
        assert_eq!(action["kind"], "redirect", "{action}");
        assert!(action.get("error").is_none(), "{action}");
        assert_eq!(
            shop.ledger.return_urls().last().cloned().flatten().as_deref(),
            Some("https://shop.example/thanks")
        );
        let created = shop.ledger.payments().len();

        // Gọi lại cùng khoá: cùng lượt thu, không gọi đối tác thêm.
        let (status, again) =
            public(shop, "POST", &uri, Some(&token), Some("r1"), Some(retry)).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"][0]["payment_id"], action["payment_id"]);
        assert_eq!(shop.ledger.payments().len(), created);
        // Cùng khoá, nội dung khác: 422.
        let (status, reused) =
            public(shop, "POST", &uri, Some(&token), Some("r1"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{reused}");
        assert_eq!(reused["code"], "orders.idempotency_key_reused");

        // Token chỉ mở đúng đơn của nó.
        let (other, _, _) = refused_order(shop, None).await;
        let (status, hidden) = public(
            shop,
            "POST",
            &format!("/public/orders/{other}/payment-link"),
            Some(&token),
            None,
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");
        assert_eq!(hidden["code"], "orders.order_not_found");

        // Quá số lần mỗi giờ: 429 kèm `Retry-After`.
        let mut limited = None;
        for attempt in 0..crate::payment_retry::RETRY_PER_HOUR {
            let (status, answer) = public(
                shop,
                "POST",
                &uri,
                Some(&token),
                Some(&format!("burst-{attempt}")),
                Some(json!({})),
            )
            .await;
            if status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(answer);
                break;
            }
            assert_eq!(status, StatusCode::OK, "{answer}");
        }
        let limited = limited.expect("the hourly limit was never reached");
        assert_eq!(limited["code"], "orders.payment_retry_too_many");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_mua_dang_nhap_tu_mo_lai_luot_thu_cua_minh() {
    TestDatabase::run(async |test| {
        let shop = super::shop(test).await;
        let (order, _, _) = refused_order(&shop, Some("buyer1")).await;
        let uri = format!("/me/orders/{order}/payment-link");
        let (status, hidden) =
            Call::as_user(&shop, "buyer2").send("POST", &uri, Some(json!({}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{hidden}");
        let (status, opened) =
            Call::as_user(&shop, "buyer1").send("POST", &uri, Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{opened}");
        assert_eq!(opened["data"][0]["kind"], "redirect", "{opened}");
        // Đơn của người mua hiện cũng báo lượt thu mới (không còn `none`).
        let (_, mine) =
            Call::as_user(&shop, "buyer1").send("GET", &format!("/me/orders/{order}"), None).await;
        assert_eq!(mine["data"]["status"], "pending_payment");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn don_khong_cho_thanh_toan_thi_khong_mo_lai() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let shop = &guests.shop;
        let (order, number, _) = refused_order(shop, None).await;
        let token = access_token(&guests, &number).await;
        let uri = format!("/public/orders/{order}/cancel");
        let (status, cancelled) =
            public(shop, "POST", &uri, Some(&token), None, Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
        let (status, refused) = public(
            shop,
            "POST",
            &format!("/public/orders/{order}/payment-link"),
            Some(&token),
            None,
            Some(json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["code"], "orders.invalid_transition");
    })
    .await
    .unwrap();
}

/// Lý do lỗi dịch theo ngôn ngữ người gọi, như `detail` của `Problem`.
#[test]
fn ly_do_loi_dich_theo_ngon_ngu() {
    use autoxide_kernel::i18n::{Language, Translations};
    let translations =
        Translations::new(vec![("orders", vec![("vi", include_str!("../../locales/vi.json"))])])
            .unwrap();
    let failure =
        crate::payments::StartFailure::of(&crate::errors::payment_method_unavailable("card"));
    let action = crate::payments::Action {
        kind: "none".to_owned(),
        redirect_url: None,
        qr_payload: None,
        transfer_code: None,
        error: Some(failure),
    };
    let now = autoxide_kernel::time::Timestamp::now();
    let row = crate::entity::payment::Model {
        id: uuid::Uuid::now_v7(),
        tenant_id: uuid::Uuid::now_v7(),
        order_id: uuid::Uuid::now_v7(),
        purpose: "order".to_owned(),
        method: "card".to_owned(),
        gateway: None,
        ledger_payment_id: None,
        external_id: None,
        reference: "r".to_owned(),
        amount_minor: 1000,
        currency: "VND".to_owned(),
        tendered_minor: None,
        status: "pending".to_owned(),
        action: Some(serde_json::to_string(&action).unwrap()),
        expires_at: None,
        succeeded_at: None,
        created_at: now,
        updated_at: now,
        installment: None,
    };
    let response = |language| {
        let built = crate::http::dto::PaymentActionResponse::new(
            &row,
            "t".to_owned(),
            &translations,
            language,
        )
        .unwrap();
        serde_json::to_value(built).unwrap()
    };
    let english = response(Language::En);
    assert_eq!(english["kind"], "none");
    assert_eq!(english["error"]["code"], "orders.payment_method_unavailable");
    assert_eq!(
        english["error"]["message"],
        "The payment method card cannot be used for this order."
    );
    let vietnamese = response(Language::Vi);
    assert_eq!(
        vietnamese["error"]["message"],
        "Phương thức thanh toán card không dùng được cho đơn này."
    );
}
