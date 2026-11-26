//! Yêu thích (gộp từ trình duyệt), vừa xem, nhắc giỏ bỏ dở chỉ khi có đồng ý quảng cáo, giới thiệu
//! bạn (bước 47 mục 5).

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use super::{Abandoned, BUYER, ConsentWithdrawn, Order, Shop, call, drain, jobs_due_now, shop};

async fn with_reminders(shop: &Shop) {
    let (status, body) = call(
        shop,
        "POST",
        "/reminder-rules",
        "boss",
        Some(json!({"delays_minutes": [60, 1440], "enabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

async fn abandon(shop: &Shop, user: u128) -> Uuid {
    let cart = Uuid::now_v7();
    shop.order_event(&Abandoned(json!({"after": {
        "cart_id": cart, "tenant_id": autoxide_kernel::tenancy::DEFAULT_TENANT, "org_id": shop.tree.a,
        "buyer_user_id": Uuid::from_u128(user), "currency": "VND", "lines": [],
    }})))
    .await;
    drain(shop).await;
    cart
}

fn reminders(shop: &Shop) -> usize {
    shop.notifier.sent().iter().filter(|sent| sent.kind == "customers.cart_reminder").count()
}

#[tokio::test]
async fn wishlist_merges_browser_items_without_duplicates() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let (status, _) = call(&shop, "POST", "/me/wishlist", "buyer", Some(json!({"product_id": a}))).await;
        assert_eq!(status, StatusCode::OK);
        let (status, merged) = call(&shop, "POST", "/me/wishlist/merge", "buyer", Some(json!({"items": [
            {"product_id": a}, {"product_id": b}, {"product_id": b}, {"product_id": a, "variant_id": Uuid::now_v7()},
        ]})))
        .await;
        assert_eq!(status, StatusCode::OK, "{merged}");
        assert_eq!(merged["data"].as_array().unwrap().len(), 3);
        let (status, _) = call(&shop, "DELETE", &format!("/me/wishlist/{a}"), "buyer", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, list) = call(&shop, "GET", "/me/wishlist", "buyer", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        // Danh sách của người khác không lẫn.
        let (_, other) = call(&shop, "GET", "/me/wishlist", "buyer2", None).await;
        assert!(other["data"].as_array().unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn recently_viewed_keeps_latest_first() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        for product in [a, b, a] {
            let (status, _) = call(
                &shop,
                "POST",
                "/me/recently-viewed",
                "buyer",
                Some(json!({"product_id": product})),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        let (_, list) = call(&shop, "GET", "/me/recently-viewed", "buyer", None).await;
        let rows = list["data"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["product_id"], json!(a));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cart_reminder_only_with_marketing_consent() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_reminders(&shop).await;
        // Không đồng ý: không lên lịch.
        abandon(&shop, BUYER).await;
        jobs_due_now(&shop).await;
        drain(&shop).await;
        assert_eq!(reminders(&shop), 0);
        // Có đồng ý: hai lần nhắc, mỗi lần một tin (job chạy lại không gửi thêm).
        super::allow_marketing(&shop.consents, 102);
        abandon(&shop, 102).await;
        jobs_due_now(&shop).await;
        drain(&shop).await;
        jobs_due_now(&shop).await;
        drain(&shop).await;
        assert_eq!(reminders(&shop), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn consent_withdrawn_cancels_pending_reminders() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_reminders(&shop).await;
        super::allow_marketing(&shop.consents, BUYER);
        abandon(&shop, BUYER).await;
        super::deny_marketing(&shop.consents, BUYER);
        shop.compliance_event(&ConsentWithdrawn(json!({"consent_id": Uuid::now_v7(), "purpose": "marketing", "scope": "", "channel": ""}))).await;
        drain(&shop).await;
        jobs_due_now(&shop).await;
        drain(&shop).await;
        assert_eq!(reminders(&shop), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn order_placed_cancels_reminder() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_reminders(&shop).await;
        super::allow_marketing(&shop.consents, BUYER);
        abandon(&shop, BUYER).await;
        let order = Order::of(BUYER, shop.tree.a, 100_000);
        shop.order_event(&order.placed()).await;
        drain(&shop).await;
        jobs_due_now(&shop).await;
        drain(&shop).await;
        assert_eq!(reminders(&shop), 0);
    })
    .await
    .unwrap();
}

async fn with_referral_rule(shop: &Shop) {
    let (status, body) = call(shop, "POST", "/loyalty/rules", "boss", Some(json!({
        "earn_per_minor": 10000, "points_per_unit": 1, "point_value_minor": 1000, "currency": "VND",
        "points_valid_months": 12, "min_redeem_points": 10, "max_redeem_ratio_bp": 5000,
    })))
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = call(shop, "POST", "/referral-rules", "boss", Some(json!({
        "referrer_points": 50, "referee_points": 20, "min_first_order_minor": 100000, "currency": "VND",
    })))
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test]
async fn referral_rewarded_once_after_first_order_completed() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_referral_rule(&shop).await;
        let (_, mine) = call(&shop, "GET", "/me/referral", "buyer2", None).await;
        let code = mine["data"]["code"].as_str().unwrap().to_owned();
        let (status, body) = call(
            &shop,
            "POST",
            "/me/referral/apply",
            "buyer",
            Some(json!({"code": code.to_lowercase()})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let (status, again) =
            call(&shop, "POST", "/me/referral/apply", "buyer", Some(json!({"code": code}))).await;
        assert_eq!(
            (status, again["code"].as_str()),
            (StatusCode::CONFLICT, Some("customers.referral_not_eligible"))
        );
        let order = Order::of(BUYER, shop.tree.a, 200_000);
        shop.order_event(&order.placed()).await;
        shop.order_event(&order.status("completed")).await;
        shop.order_event(&order.status("completed")).await;
        drain(&shop).await;
        let (_, referee) = call(&shop, "GET", "/me/points", "buyer", None).await;
        assert_eq!(referee["data"]["balance"], 20 + 20, "{referee}"); // thưởng + tích của đơn
        let (_, referrer) = call(&shop, "GET", "/me/points", "buyer2", None).await;
        assert_eq!(referrer["data"]["balance"], 50);
        let (_, mine) = call(&shop, "GET", "/me/referral", "buyer2", None).await;
        assert_eq!(mine["data"]["rewarded"], 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn referral_reward_taken_back_when_qualifying_order_refunded_below_minimum() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_referral_rule(&shop).await;
        let (_, mine) = call(&shop, "GET", "/me/referral", "buyer2", None).await;
        let code = mine["data"]["code"].as_str().unwrap().to_owned();
        let (status, body) =
            call(&shop, "POST", "/me/referral/apply", "buyer", Some(json!({"code": code}))).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let order = Order::of(BUYER, shop.tree.a, 200_000);
        shop.order_event(&order.placed()).await;
        shop.order_event(&order.status("completed")).await;
        drain(&shop).await;
        // Hoàn một phần, đơn còn trên mức tối thiểu (100.000): thưởng giữ nguyên.
        shop.order_event(&order.refunded(Uuid::now_v7(), 50_000)).await;
        drain(&shop).await;
        let (_, referrer) = call(&shop, "GET", "/me/points", "buyer2", None).await;
        assert_eq!(referrer["data"]["balance"], 50, "{referrer}");
        // Hoàn tiếp, còn 50.000 < mức tối thiểu: thu hồi thưởng hai bên; event tới lại không thu thêm.
        let refund = order.refunded(Uuid::now_v7(), 100_000);
        shop.order_event(&refund).await;
        shop.order_event(&refund).await;
        drain(&shop).await;
        let (_, referrer) = call(&shop, "GET", "/me/points", "buyer2", None).await;
        assert_eq!(referrer["data"]["balance"], 0, "{referrer}");
        let (_, referee) = call(&shop, "GET", "/me/points", "buyer", None).await;
        // Điểm tích của đơn (20) thu hồi theo tỉ lệ tiền hoàn: còn 20 − 5 − 10 = 5; thưởng 20 thu hết.
        assert_eq!(referee["data"]["balance"], 5, "{referee}");
        let (_, mine) = call(&shop, "GET", "/me/referral", "buyer2", None).await;
        assert_eq!(mine["data"]["rewarded"], 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn self_referral_same_phone_rejected() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_referral_rule(&shop).await;
        let (_, mine) = call(&shop, "GET", "/me/referral", "buyer", None).await;
        let code = mine["data"]["code"].as_str().unwrap().to_owned();
        let (status, problem) =
            call(&shop, "POST", "/me/referral/apply", "buyer", Some(json!({"code": code}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("customers.self_referral"))
        );
        // Hai tài khoản cùng email.
        call(&shop, "PUT", "/me", "buyer", Some(json!({"email": "an@example.com"}))).await;
        call(&shop, "PUT", "/me", "buyer3", Some(json!({"email": "AN@example.com"}))).await;
        let (status, problem) =
            call(&shop, "POST", "/me/referral/apply", "buyer3", Some(json!({"code": code}))).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("customers.self_referral"))
        );
        let (status, problem) =
            call(&shop, "POST", "/me/referral/apply", "buyer2", Some(json!({"code": "ZZZZZZZZ"})))
                .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("customers.referral_code_invalid"))
        );
    })
    .await
    .unwrap();
}

// Duyệt hết danh sách `first` (đã có `?limit=…`) theo `next_cursor`; trả trường `field` theo thứ tự nhận.
async fn walk(shop: &Shop, first: &str, who: &str, field: &str) -> Vec<String> {
    let mut seen = Vec::new();
    let mut uri = first.to_owned();
    loop {
        let (status, page) = call(shop, "GET", &uri, who, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {page}");
        let rows = page["data"].as_array().unwrap();
        assert!(rows.len() <= 2, "{page}");
        seen.extend(rows.iter().map(|row| row[field].as_str().unwrap().to_owned()));
        match page["pagination"]["next_cursor"].as_str() {
            Some(cursor) => uri = format!("{first}&cursor={cursor}"),
            None => return seen,
        }
    }
}

/// Bước 70 P9 (D-20261125-list-pages): vừa xem (mới xem trước), quy tắc nhắc giỏ và quy tắc giới thiệu (mới
/// nhất trước) là một trang; theo `next_cursor` tới hết không trùng không sót, `order=asc` ngược lại.
#[tokio::test]
async fn vua_xem_va_quy_tac_phan_trang() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let products: Vec<Uuid> = (0..5).map(|_| Uuid::now_v7()).collect();
        for product in &products {
            let body = Some(json!({"product_id": product}));
            let (status, _) = call(&shop, "POST", "/me/recently-viewed", "buyer", body).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        let mut viewed: Vec<String> = products.iter().map(Uuid::to_string).collect();
        assert_eq!(walk(&shop, "/me/recently-viewed?limit=2&order=asc", "buyer", "product_id").await, viewed);
        viewed.reverse();
        assert_eq!(walk(&shop, "/me/recently-viewed?limit=2", "buyer", "product_id").await, viewed);

        for (path, body) in [
            ("/reminder-rules", json!({"delays_minutes": [60], "enabled": true})),
            ("/referral-rules", json!({"referrer_points": 5, "referee_points": 1, "min_first_order_minor": 0, "currency": "VND"})),
        ] {
            let mut ids = Vec::new();
            for _ in 0..5 {
                let (status, made) = call(&shop, "POST", path, "boss", Some(body.clone())).await;
                assert_eq!(status, StatusCode::CREATED, "{made}");
                ids.push(made["data"]["id"].as_str().unwrap().to_owned());
            }
            assert_eq!(walk(&shop, &format!("{path}?limit=2&order=asc"), "boss", "id").await, ids);
            ids.reverse();
            assert_eq!(walk(&shop, &format!("{path}?limit=2"), "boss", "id").await, ids);
        }
        let (status, _) = call(&shop, "GET", "/reminder-rules?order=up", "boss", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}

/// `GET /me/wishlist` theo trang (bước 70 P9): thêm gần nhất trước, trang 2 tới hết không trùng không sót,
/// `order=asc` theo thứ tự thêm; dòng mới có ID (cursor theo ID).
#[tokio::test]
async fn wishlist_pages_newest_first() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let products: Vec<Uuid> = (0..5).map(|_| Uuid::now_v7()).collect();
        for product in &products {
            let (status, body) =
                call(&shop, "POST", "/me/wishlist", "buyer", Some(json!({"product_id": product})))
                    .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        let walk = async |query: &str| {
            let mut found = Vec::new();
            let mut uri = format!("/me/wishlist?limit=2{query}");
            loop {
                let (status, page) = call(&shop, "GET", &uri, "buyer", None).await;
                assert_eq!(status, StatusCode::OK, "{page}");
                let rows = page["data"].as_array().unwrap();
                assert!(rows.len() <= 2);
                found.extend(
                    rows.iter()
                        .map(|row| row["product_id"].as_str().unwrap().parse::<Uuid>().unwrap()),
                );
                match page["pagination"]["next_cursor"].as_str() {
                    Some(cursor) => uri = format!("/me/wishlist?limit=2{query}&cursor={cursor}"),
                    None => return found,
                }
            }
        };
        assert_eq!(walk("").await, products.iter().rev().copied().collect::<Vec<_>>());
        assert_eq!(walk("&order=asc").await, products);
        let (status, counted) = call(&shop, "GET", "/me/wishlist?count=true", "buyer", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(counted["pagination"]["total"], 5);
        let (status, _) = call(&shop, "GET", "/me/wishlist?limit=0", "buyer", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    })
    .await
    .unwrap();
}
