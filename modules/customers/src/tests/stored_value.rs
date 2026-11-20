//! Thẻ quà tặng / số dư cửa hàng (bước 47 mục 4): tắt mặc định, không có đường rút tiền, tra mã sai
//! bị giới hạn, trả đồng thời không vượt số dư, giữ / xác nhận / trả theo đơn.

use autoxide_kernel::money::{Currency, Money};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{BUYER, Order, Shop, call, drain, minor, shop, shop_with};

const ON: (&str, &str) = ("AUTOXIDE_CUSTOMERS_STORED_VALUE_ENABLED", "true");

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::parse("VND").unwrap())
}

fn amount(minor: i64) -> Value {
    json!({"amount": minor.to_string(), "currency": "VND"})
}

/// Phát hành thẻ `value` với tham chiếu `reference`; trả thân `data`.
async fn issue(shop: &Shop, value: i64, reference: &str) -> Value {
    let (status, body) = call(
        shop,
        "POST",
        "/gift-cards",
        "boss",
        Some(json!({"amount": amount(value), "reference": reference})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["data"].clone()
}

async fn profile_id(shop: &Shop) -> String {
    let (_, me) = call(shop, "GET", "/me", "buyer", None).await;
    me["data"]["id"].as_str().unwrap().to_owned()
}

async fn card(shop: &Shop, id: &str) -> Value {
    let (status, body) = call(shop, "GET", &format!("/gift-cards/{id}"), "boss", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"].clone()
}

#[tokio::test]
async fn stored_value_disabled_by_default() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let id = Uuid::now_v7();
        let calls = [
            (
                "POST",
                "/gift-cards".to_owned(),
                "boss",
                Some(json!({"amount": amount(1000), "reference": "r"})),
            ),
            ("GET", format!("/gift-cards/{id}"), "boss", None),
            ("POST", "/gift-cards/lookup".to_owned(), "boss", Some(json!({"code": "X"}))),
            ("POST", format!("/gift-cards/{id}/void"), "boss", None),
            ("GET", "/me/gift-cards".to_owned(), "buyer", None),
            ("POST", "/me/gift-cards/attach".to_owned(), "buyer", Some(json!({"code": "X"}))),
            ("GET", "/me/store-credit".to_owned(), "buyer", None),
        ];
        for (method, uri, who, body) in calls {
            let (status, answer) = call(&shop, method, &uri, who, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {answer}");
            assert_eq!(answer["code"], "customers.stored_value_disabled", "{method} {uri}");
        }
        // Tắt thì `orders` không nhận hình thức trả bằng thẻ hay số dư.
        assert!(shop.tender("gift_card").is_none());
        assert!(shop.tender("store_credit").is_none());
        // `/me/store-credit` bị chặn trước khi tạo hồ sơ: không để lại dấu vết.
        let (_, list) = call(&shop, "GET", "", "boss", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 0, "{list}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gift_card_has_no_cash_out_path() {
    TestDatabase::run(async |test| {
        let shop = shop_with(test, &[ON]).await;
        let paths: Vec<String> = shop.openapi().paths.paths.keys().cloned().collect();
        let stored: Vec<&String> = paths
            .iter()
            .filter(|path| path.contains("gift-card") || path.contains("store-credit"))
            .collect();
        assert!(!stored.is_empty(), "{paths:?}");
        for path in stored {
            for word in ["withdraw", "cash", "payout", "transfer", "top-up", "topup", "refund"] {
                assert!(!path.contains(word), "{path} looks like a cash-out path");
            }
        }
        // Không tool MCP nào chạm thẻ / số dư.
        let client = shop.mcp("boss").await.unwrap();
        let tools = client.list_all_tools().await.unwrap();
        assert!(
            tools.iter().all(|tool| !tool.name.contains("gift") && !tool.name.contains("credit")),
            "{tools:?}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gift_card_issue_lookup_attach_void() {
    TestDatabase::run(async |test| {
        let shop = shop_with(test, &[ON]).await;
        let issued = issue(&shop, 500_000, "POS-1").await;
        let code = issued["code"].as_str().unwrap().to_owned();
        assert_eq!(code.len(), 16);
        assert_eq!(issued["code_last4"], code[12..]);
        assert_eq!(minor(&issued["balance"]), 500_000);

        // Cùng tham chiếu: trả thẻ cũ, không thẻ mới, không lộ mã lần hai.
        let again = issue(&shop, 500_000, "POS-1").await;
        assert_eq!(again["id"], issued["id"]);
        assert!(again.get("code").is_none(), "{again}");
        let id = issued["id"].as_str().unwrap();
        assert!(card(&shop, id).await.get("code").is_none());

        // Tra theo mã: bỏ qua khoảng trắng, gạch, chữ thường.
        let messy = format!("{}-{} {}", &code[..4], &code[4..8], code[8..].to_lowercase());
        let (status, found) =
            call(&shop, "POST", "/gift-cards/lookup", "boss", Some(json!({"code": messy}))).await;
        assert_eq!(status, StatusCode::OK, "{found}");
        assert_eq!(found["data"]["id"], issued["id"]);

        // Vượt mệnh giá tối đa.
        let (status, body) = call(
            &shop,
            "POST",
            "/gift-cards",
            "boss",
            Some(json!({"amount": amount(20_000_000), "reference": "POS-2"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["code"], "customers.face_value_too_high");

        // Không có quyền: 403 (dù đã bật).
        let (status, _) = call(&shop, "GET", &format!("/gift-cards/{id}"), "reader", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Người mua gắn thẻ vào tài khoản.
        let (status, body) =
            call(&shop, "POST", "/me/gift-cards/attach", "buyer", Some(json!({"code": code})))
                .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (_, mine) = call(&shop, "GET", "/me/gift-cards", "buyer", None).await;
        assert_eq!(mine["data"][0]["id"], issued["id"], "{mine}");
        let (_, theirs) = call(&shop, "GET", "/me/gift-cards", "buyer2", None).await;
        assert_eq!(theirs["data"].as_array().unwrap().len(), 0);

        // Huỷ: số dư về 0, huỷ lần hai 409, trả bằng thẻ đã huỷ bị từ chối.
        let (status, voided) =
            call(&shop, "POST", &format!("/gift-cards/{id}/void"), "boss", None).await;
        assert_eq!(status, StatusCode::OK, "{voided}");
        assert_eq!(
            (voided["data"]["status"].as_str(), minor(&voided["data"]["balance"])),
            (Some("void"), 0)
        );
        let (status, body) =
            call(&shop, "POST", &format!("/gift-cards/{id}/void"), "boss", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::CONFLICT, Some("customers.gift_card_inactive"))
        );
        let error = shop
            .tender("gift_card")
            .unwrap()
            .hold(
                &shop.default_tenant(),
                "orders:void",
                Some(Uuid::from_u128(BUYER)),
                &json!({"code": code}),
                vnd(100_000),
                Utc::now() + Duration::minutes(30),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "customers.gift_card_inactive");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gift_card_lookup_throttled_on_wrong_codes() {
    TestDatabase::run(async |test| {
        let shop =
            shop_with(test, &[ON, ("AUTOXIDE_CUSTOMERS_GIFT_CARD_LOOKUPS_PER_HOUR", "3")]).await;
        let code = issue(&shop, 100_000, "POS-1").await["code"].as_str().unwrap().to_owned();
        let lookup = async |code: &str| {
            call(&shop, "POST", "/me/gift-cards/attach", "buyer", Some(json!({"code": code}))).await
        };
        // Tra đúng không bị đếm.
        for _ in 0..5 {
            assert_eq!(lookup(&code).await.0, StatusCode::OK);
        }
        for _ in 0..3 {
            let (status, body) = lookup("WRONGWRONGWRONG2").await;
            assert_eq!(
                (status, body["code"].as_str()),
                (StatusCode::NOT_FOUND, Some("customers.gift_card_not_found"))
            );
        }
        // Lần sai thứ tư vượt giới hạn; sau đó cả mã đúng cũng bị chặn trong giờ đó.
        let (status, _) = lookup("WRONGWRONGWRONG2").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, body) = lookup(&code).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
        assert_eq!(body["code"], "customers.lookup_throttled");
        // Người khác không bị ảnh hưởng.
        let (status, _) =
            call(&shop, "POST", "/me/gift-cards/attach", "buyer2", Some(json!({"code": code})))
                .await;
        assert_eq!(status, StatusCode::OK);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn concurrent_gift_card_redemptions_never_overdraw() {
    TestDatabase::run(async |test| {
        let shop = shop_with(test, &[ON]).await;
        let issued = issue(&shop, 100_000, "POS-1").await;
        let code = issued["code"].as_str().unwrap().to_owned();
        let tenant = shop.default_tenant();
        let mut tasks = tokio::task::JoinSet::new();
        for buyer in 0..10_u128 {
            let tender = shop.tender("gift_card").unwrap();
            let tenant = tenant.clone();
            let code = code.clone();
            tasks.spawn(async move {
                tender
                    .hold(
                        &tenant,
                        &format!("orders:{}", Uuid::now_v7()),
                        Some(Uuid::from_u128(500 + buyer)),
                        &json!({"code": code}),
                        vnd(30_000),
                        Utc::now() + Duration::minutes(30),
                    )
                    .await
            });
        }
        let mut held = 0;
        while let Some(result) = tasks.join_next().await {
            match result.unwrap() {
                Ok(money) => held += money.minor(),
                Err(error) => {
                    assert_eq!(error.code(), "customers.insufficient_balance", "{error:?}");
                }
            }
        }
        let id = issued["id"].as_str().unwrap();
        let after = card(&shop, id).await;
        // Mỗi lần giữ tối đa phần còn lại: tổng giữ + số dư đúng bằng mệnh giá, số dư không âm.
        assert_eq!(held + minor(&after["balance"]), 100_000, "{after}");
        assert!(minor(&after["balance"]) >= 0);
        assert!(held >= 90_000, "held {held}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gift_card_hold_confirmed_or_released_with_order() {
    TestDatabase::run(async |test| {
        let shop = shop_with(test, &[ON]).await;
        let issued = issue(&shop, 100_000, "POS-1").await;
        let code = issued["code"].as_str().unwrap();
        let id = issued["id"].as_str().unwrap();
        let tenant = shop.default_tenant();
        let tender = shop.tender("gift_card").unwrap();
        let buyer = Some(Uuid::from_u128(BUYER));
        let until = Utc::now() + Duration::minutes(30);

        // Khách vãng lai không trả được bằng thẻ.
        let error = tender
            .hold(&tenant, "orders:guest", None, &json!({"code": code}), vnd(10_000), until)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "customers.stored_value_need_account");

        // Huỷ đơn → tiền về thẻ; gọi lại cùng đơn không giữ thêm.
        let cancelled = Order::of(BUYER, shop.tree.a, 40_000);
        let reference = format!("orders:{}", cancelled.id);
        for _ in 0..2 {
            let held = tender
                .hold(&tenant, &reference, buyer, &json!({"code": code}), vnd(40_000), until)
                .await
                .unwrap();
            assert_eq!(held, vnd(40_000));
        }
        assert_eq!(minor(&card(&shop, id).await["balance"]), 60_000);
        shop.order_event(&cancelled.cancelled()).await;
        drain(&shop).await;
        assert_eq!(minor(&card(&shop, id).await["balance"]), 100_000);

        // Đơn xác nhận → tiền mất thật.
        let paid = Order::of(BUYER, shop.tree.a, 70_000);
        tender
            .hold(
                &tenant,
                &format!("orders:{}", paid.id),
                buyer,
                &json!({"code": code}),
                vnd(70_000),
                until,
            )
            .await
            .unwrap();
        shop.order_event(&paid.confirmed()).await;
        drain(&shop).await;
        assert_eq!(minor(&card(&shop, id).await["balance"]), 30_000);
        // Event lặp lại không trừ lần hai.
        shop.order_event(&paid.cancelled()).await;
        drain(&shop).await;
        assert_eq!(minor(&card(&shop, id).await["balance"]), 30_000);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn store_credit_adjusted_by_staff_and_spent_once() {
    TestDatabase::run(async |test| {
        let shop = shop_with(test, &[ON]).await;
        let profile = profile_id(&shop).await;
        let adjust = async |value: i64, reference: &str| {
            call(
                &shop,
                "POST",
                &format!("/{profile}/store-credit/adjustments"),
                "boss",
                Some(json!({"amount": amount(value), "reference": reference, "reason": "late delivery"})),
            )
            .await
        };
        let (status, body) = adjust(50_000, "TICKET-1").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Cùng tham chiếu: không cộng lần hai.
        let (_, body) = adjust(50_000, "TICKET-1").await;
        assert_eq!(minor(&body["data"]["balance"]), 50_000);
        let (status, body) = adjust(-80_000, "TICKET-2").await;
        assert_eq!((status, body["code"].as_str()), (StatusCode::CONFLICT, Some("customers.insufficient_balance")));

        let (_, mine) = call(&shop, "GET", "/me/store-credit", "buyer", None).await;
        assert_eq!(minor(&mine["data"]["balance"]), 50_000, "{mine}");
        let (_, none) = call(&shop, "GET", "/me/store-credit", "buyer2", None).await;
        assert!(none["data"].is_null(), "{none}");

        // Trả đơn 30.000 rồi đơn 30.000: lần hai chỉ còn 20.000.
        let tender = shop.tender("store_credit").unwrap();
        let tenant = shop.default_tenant();
        let until = Utc::now() + Duration::minutes(30);
        let buyer = Some(Uuid::from_u128(BUYER));
        let first = tender.hold(&tenant, "orders:a", buyer, &json!({}), vnd(30_000), until).await.unwrap();
        let second = tender.hold(&tenant, "orders:b", buyer, &json!({}), vnd(30_000), until).await.unwrap();
        assert_eq!((first, second), (vnd(30_000), vnd(20_000)));
        let error = tender.hold(&tenant, "orders:c", buyer, &json!({}), vnd(30_000), until).await.unwrap_err();
        assert_eq!(error.code(), "customers.insufficient_balance");
        tender.release(&tenant, "orders:b").await.unwrap();
        let (_, mine) = call(&shop, "GET", "/me/store-credit", "buyer", None).await;
        assert_eq!(minor(&mine["data"]["balance"]), 20_000, "{mine}");
    })
    .await
    .unwrap();
}
