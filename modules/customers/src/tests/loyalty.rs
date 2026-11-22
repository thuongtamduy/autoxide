//! Điểm thưởng, hạng, voucher tặng (bước 47 mục 3, 6): tích khi hoàn tất, lô hết hạn trước tiêu
//! trước, giữ / trả / xác nhận khi trả đơn bằng điểm, đổi đồng thời không âm, nghĩa vụ khớp sổ cái.

use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::root_of;
use autoxide_kernel::time::Timestamp;
use autoxide_ledger::api::{AccountKey, Ledger};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{BUYER, Order, Shop, age_points, call, drain, shop};

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::parse("VND").unwrap())
}

/// 10.000 đ tiền hàng = 1 điểm; 1 điểm = 1.000 đ; hạn 12 tháng; đổi tối thiểu 10, tối đa 50% đơn.
async fn with_rule(shop: &Shop) {
    let (status, body) = call(shop, "POST", "/loyalty/rules", "boss", Some(json!({
        "earn_per_minor": 10000, "points_per_unit": 1, "point_value_minor": 1000, "currency": "VND",
        "points_valid_months": 12, "min_redeem_points": 10, "max_redeem_ratio_bp": 5000,
    })))
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// Đơn của người mua `buyer` tiền hàng `goods`, đặt rồi hoàn tất.
async fn completed_order(shop: &Shop, goods: i64) -> Order {
    let order = Order::of(BUYER, shop.tree.a, goods);
    shop.order_event(&order.placed()).await;
    shop.order_event(&order.status("completed")).await;
    drain(shop).await;
    order
}

async fn my_points(shop: &Shop) -> Value {
    let (status, body) = call(shop, "GET", "/me/points", "buyer", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"].clone()
}

async fn profile_id(shop: &Shop) -> String {
    let (_, me) = call(shop, "GET", "/me", "buyer", None).await;
    me["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn points_earned_on_completion_not_on_payment() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_rule(&shop).await;
        let order = Order::of(BUYER, shop.tree.a, 300_000);
        shop.order_event(&order.placed()).await;
        shop.order_event(&order.status("confirmed")).await;
        shop.order_event(&order.confirmed()).await;
        shop.order_event(&order.status("delivered")).await;
        drain(&shop).await;
        assert_eq!(my_points(&shop).await["balance"], 0);
        // Hoàn tất (giao hai lần): 30 điểm, một lần.
        shop.order_event(&order.status("completed")).await;
        shop.order_event(&order.status("completed")).await;
        drain(&shop).await;
        let points = my_points(&shop).await;
        assert_eq!(points["balance"], 30, "{points}");
        assert_eq!(points["lots"].as_array().unwrap().len(), 1);
        assert_eq!(points["rule"]["point_value"]["amount"], "1000");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn points_expire_fifo_by_lot() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_rule(&shop).await;
        let id = profile_id(&shop).await;
        for (points, reference) in [(50, "a"), (30, "b")] {
            let (status, body) = call(
                &shop,
                "POST",
                &format!("/{id}/points/adjustments"),
                "boss",
                Some(json!({
                    "points": points, "reference": reference, "reason": "bù đơn giao trễ",
                })),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        // Cùng tham chiếu lần hai: không cộng thêm.
        call(
            &shop,
            "POST",
            &format!("/{id}/points/adjustments"),
            "boss",
            Some(json!({
                "points": 50, "reference": "a", "reason": "gửi lại",
            })),
        )
        .await;
        let (_, after) = call(
            &shop,
            "POST",
            &format!("/{id}/points/adjustments"),
            "boss",
            Some(json!({
                "points": -60, "reference": "c", "reason": "thu hồi",
            })),
        )
        .await;
        // Lô cũ nhất tiêu trước: lô 50 hết, lô 30 còn 20.
        let lots = after["data"]["lots"].as_array().unwrap();
        assert_eq!(lots.len(), 1, "{after}");
        assert_eq!(
            (lots[0]["earned"].as_i64(), lots[0]["remaining"].as_i64()),
            (Some(30), Some(20))
        );
        let (status, problem) = call(
            &shop,
            "POST",
            &format!("/{id}/points/adjustments"),
            "boss",
            Some(json!({
                "points": -21, "reference": "d", "reason": "quá số dư",
            })),
        )
        .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("customers.insufficient_points"))
        );

        age_points(&shop).await;
        shop.run_job(crate::jobs::EXPIRE_POINTS).await;
        shop.run_job(crate::jobs::EXPIRE_POINTS).await;
        let points = my_points(&shop).await;
        assert_eq!(
            (points["balance"].as_i64(), points["held"].as_i64()),
            (Some(0), Some(0)),
            "{points}"
        );
        let (_, entries) = call(&shop, "GET", &format!("/{id}/points/entries"), "boss", None).await;
        let expired: Vec<&Value> = entries["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["kind"] == "expire")
            .collect();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0]["points"], -20);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn points_hold_released_when_order_cancelled_or_hold_expires() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_rule(&shop).await;
        completed_order(&shop, 1_000_000).await;
        let tenant = shop.default_tenant();
        let tender = shop.points_tender();
        let buyer = Some(Uuid::from_u128(BUYER));
        let until = Timestamp::now() + SignedDuration::from_mins(30);

        // Đơn bị huỷ → điểm về.
        let cancelled = Order::of(BUYER, shop.tree.a, 200_000);
        let reference = format!("orders:{}", cancelled.id);
        let held = tender
            .hold(&tenant, &reference, buyer, &json!({"points": 20}), vnd(200_000), until)
            .await
            .unwrap();
        assert_eq!(held, vnd(20_000));
        // Gọi lại cùng tham chiếu: kết quả cũ, không giữ thêm.
        tender
            .hold(&tenant, &reference, buyer, &json!({"points": 20}), vnd(200_000), until)
            .await
            .unwrap();
        let points = my_points(&shop).await;
        assert_eq!((points["balance"].as_i64(), points["held"].as_i64()), (Some(80), Some(20)));
        shop.order_event(&cancelled.cancelled()).await;
        drain(&shop).await;
        let points = my_points(&shop).await;
        assert_eq!((points["balance"].as_i64(), points["held"].as_i64()), (Some(100), Some(0)));

        // Đơn được xác nhận → điểm mất thật.
        let paid = Order::of(BUYER, shop.tree.a, 200_000);
        tender
            .hold(
                &tenant,
                &format!("orders:{}", paid.id),
                buyer,
                &json!({"points": 30}),
                vnd(200_000),
                until,
            )
            .await
            .unwrap();
        shop.order_event(&paid.confirmed()).await;
        drain(&shop).await;
        let points = my_points(&shop).await;
        assert_eq!((points["balance"].as_i64(), points["held"].as_i64()), (Some(70), Some(0)));
        assert_eq!(points["lots"][0]["remaining"], 70);

        // Giữ quá hạn → job trả lại.
        let abandoned = Order::of(BUYER, shop.tree.a, 200_000);
        tender
            .hold(
                &tenant,
                &format!("orders:{}", abandoned.id),
                buyer,
                &json!({"points": 40}),
                vnd(200_000),
                until,
            )
            .await
            .unwrap();
        age_points(&shop).await;
        shop.run_job(crate::jobs::RELEASE_EXPIRED_HOLDS).await;
        let points = my_points(&shop).await;
        assert_eq!(points["held"], 0, "{points}");

        // Luật đổi: tối thiểu 10 điểm, tối đa 50% đơn.
        let error = tender
            .hold(&tenant, "orders:x", buyer, &json!({"points": 5}), vnd(200_000), until)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "customers.redeem_below_minimum");
        let error = tender
            .hold(&tenant, "orders:y", buyer, &json!({"points": 60}), vnd(100_000), until)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "customers.redeem_above_limit");
        let error = tender
            .hold(&tenant, "orders:z", None, &json!({"points": 10}), vnd(100_000), until)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "customers.points_need_account");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn concurrent_redemptions_never_overdraw() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_rule(&shop).await;
        completed_order(&shop, 1_000_000).await; // 100 điểm
        let tenant = shop.default_tenant();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..20 {
            let tender = shop.points_tender();
            let tenant = tenant.clone();
            tasks.spawn(async move {
                let reference = format!("orders:{}", Uuid::now_v7());
                let until = Timestamp::now() + SignedDuration::from_mins(30);
                tender
                    .hold(
                        &tenant,
                        &reference,
                        Some(Uuid::from_u128(BUYER)),
                        &json!({"points": 15}),
                        vnd(1_000_000),
                        until,
                    )
                    .await
            });
        }
        let mut won = 0;
        while let Some(result) = tasks.join_next().await {
            match result.unwrap() {
                Ok(_) => won += 1,
                Err(error) => {
                    assert_eq!(error.code(), "customers.insufficient_points", "{error:?}")
                }
            }
        }
        // 100 điểm / 15 = 6 lần.
        assert_eq!(won, 6);
        let points = my_points(&shop).await;
        assert_eq!((points["balance"].as_i64(), points["held"].as_i64()), (Some(10), Some(90)));
        let lot = &points["lots"][0];
        assert_eq!((lot["remaining"].as_i64(), lot["held"].as_i64()), (Some(100), Some(90)));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn points_ledger_liability_equals_outstanding_points_value() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_rule(&shop).await;
        let id = profile_id(&shop).await;
        let order = completed_order(&shop, 3_000_000).await; // 300 điểm × 1.000
        let tenant = shop.default_tenant();
        let paid = Order::of(BUYER, shop.tree.a, 1_000_000);
        let until = Timestamp::now() + SignedDuration::from_mins(30);
        shop.points_tender()
            .hold(
                &tenant,
                &format!("orders:{}", paid.id),
                Some(Uuid::from_u128(BUYER)),
                &json!({"points": 100}),
                vnd(1_000_000),
                until,
            )
            .await
            .unwrap();
        shop.order_event(&paid.confirmed()).await; // còn 200
        call(
            &shop,
            "POST",
            &format!("/{id}/points/adjustments"),
            "boss",
            Some(json!({
                "points": 50, "reference": "bu", "reason": "bù",
            })),
        )
        .await; // 250
        // Hoàn 1/3 đơn đã tích: thu hồi 100 điểm của lô đơn (lô còn 200 chưa dùng) → 150.
        shop.order_event(&order.refunded(Uuid::now_v7(), 1_000_000)).await;
        drain(&shop).await;
        let points = my_points(&shop).await;
        assert_eq!(points["balance"], 150, "{points}");
        let outstanding: i64 = points["lots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|lot| lot["remaining"].as_i64().unwrap() * 1000)
            .sum();
        let ledger = shop.api::<dyn Ledger>().unwrap();
        let currency = Currency::parse("VND").unwrap();
        let book = root_of(tenant.id());
        let liability = ledger
            .balance(&tenant, AccountKey::new(book, "loyalty.liability", currency), None)
            .await
            .unwrap();
        assert_eq!(liability.minor(), outstanding);
        assert_eq!(liability.minor(), 150_000);
        let prepaid = ledger
            .balance(&tenant, AccountKey::new(book, "buyer.prepayments", currency), None)
            .await
            .unwrap();
        assert_eq!(prepaid.minor(), 100_000);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tier_multiplies_points_and_feeds_orders() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_rule(&shop).await;
        let (status, body) = call(
            &shop,
            "POST",
            "/loyalty/tiers",
            "boss",
            Some(json!({
                "code": "gold", "name": "Vàng", "threshold_minor": 1000000, "currency": "VND",
                "earn_multiplier_bp": 20000, "rank": 2,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        completed_order(&shop, 500_000).await; // chưa đủ hạng: 50 điểm
        completed_order(&shop, 600_000).await; // đạt hạng: 60 × 2 = 120 điểm
        assert_eq!(my_points(&shop).await["balance"], 170);
        let (_, me) = call(&shop, "GET", "/me", "buyer", None).await;
        assert_eq!(me["data"]["tier"], "gold");
        let profile = shop
            .buyer_groups()
            .profile_of(&shop.default_tenant(), Uuid::from_u128(BUYER))
            .await
            .unwrap();
        assert_eq!(profile.membership_tier.as_deref(), Some("gold"));
        // Không sửa quá khứ.
        let (status, problem) = call(
            &shop,
            "POST",
            "/loyalty/tiers",
            "boss",
            Some(json!({
                "code": "gold", "name": "Vàng", "threshold_minor": 1, "currency": "VND",
                "earn_multiplier_bp": 10000, "rank": 2, "effective_from": "2020-01-01T00:00:00Z",
            })),
        )
        .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("customers.rule_effective_in_past"))
        );
    })
    .await
    .unwrap();
}

async fn reward(shop: &Shop, kind: &str, extra: Value) -> String {
    let mut body = json!({"kind": kind, "promotion_id": Uuid::now_v7()});
    body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let (status, created) =
        call(shop, "POST", "/loyalty/voucher-rewards", "boss", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn voucher_sinh_nhat_moi_nam_mot_lan() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        reward(&shop, "birthday", json!({})).await;
        let today = (Timestamp::now() + SignedDuration::from_hours(7)).date_utc();
        let birthday = format!("1990-{:02}-{:02}", today.month(), today.day());
        let (status, body) =
            call(&shop, "PUT", "/me", "buyer", Some(json!({"birthday": birthday}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        shop.run_job(crate::jobs::BIRTHDAY_VOUCHERS).await;
        shop.run_job(crate::jobs::BIRTHDAY_VOUCHERS).await;
        let granted = shop.promotions.granted.lock().unwrap().clone();
        assert_eq!(granted.len(), 1);
        assert_eq!(granted[0].source_ref, format!("birthday:{}", today.year()));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn voucher_sau_khi_mua_mot_lan() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        reward(&shop, "purchase", json!({"min_order_minor": 200000})).await;
        let small = completed_order(&shop, 100_000).await;
        let big = completed_order(&shop, 300_000).await;
        shop.order_event(&big.status("completed")).await;
        drain(&shop).await;
        let granted = shop.promotions.granted.lock().unwrap().clone();
        assert_eq!(granted.len(), 1);
        assert_eq!(granted[0].source_ref, format!("purchase:{}", big.id));
        assert_ne!(granted[0].source_ref, format!("purchase:{}", small.id));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_diem_lay_voucher_khong_nhan_doi() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_rule(&shop).await;
        let id = reward(&shop, "points", json!({"points_cost": 50})).await;
        completed_order(&shop, 1_000_000).await; // 100 điểm
        for expected in [50, 0] {
            let (status, body) =
                call(&shop, "POST", "/me/points/vouchers", "buyer", Some(json!({"reward_id": id})))
                    .await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            assert_eq!(my_points(&shop).await["balance"], expected);
        }
        let (status, problem) =
            call(&shop, "POST", "/me/points/vouchers", "buyer", Some(json!({"reward_id": id})))
                .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("customers.insufficient_points"))
        );
        assert_eq!(shop.promotions.granted.lock().unwrap().len(), 2);
        let (_, rewards) = call(&shop, "GET", "/me/points/rewards", "buyer", None).await;
        assert_eq!(rewards["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cap_voucher_loi_hoan_diem() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        with_rule(&shop).await;
        let id = reward(&shop, "points", json!({"points_cost": 50})).await;
        completed_order(&shop, 1_000_000).await;
        *shop.promotions.fail.lock().unwrap() = true;
        let (status, _) =
            call(&shop, "POST", "/me/points/vouchers", "buyer", Some(json!({"reward_id": id})))
                .await;
        assert!(status.is_server_error(), "{status}");
        assert_eq!(my_points(&shop).await["balance"], 100);
        assert!(shop.promotions.granted.lock().unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bao_gia_gio_biet_so_diem_toi_da_dung_duoc() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let tenant = shop.default_tenant();
        let tender = shop.points_tender();
        let buyer = Some(Uuid::from_u128(BUYER));
        // Chưa có chương trình: không báo.
        assert!(tender.limit(&tenant, buyer, vnd(200_000)).await.unwrap().is_none());
        with_rule(&shop).await;
        completed_order(&shop, 1_000_000).await; // 100 điểm
        // Khách vãng lai: không báo.
        assert!(tender.limit(&tenant, None, vnd(200_000)).await.unwrap().is_none());
        // Trần 50% của 200.000 đ = 100 điểm; có đủ 100.
        let limit = tender.limit(&tenant, buyer, vnd(200_000)).await.unwrap().unwrap();
        assert_eq!((limit.amount, limit.units), (vnd(100_000), Some(100)));
        // Trần 50% của 30.000 đ = 15 điểm.
        let limit = tender.limit(&tenant, buyer, vnd(30_000)).await.unwrap().unwrap();
        assert_eq!((limit.amount, limit.units), (vnd(15_000), Some(15)));
        // Dưới mức đổi tối thiểu (10 điểm): 0.
        let limit = tender.limit(&tenant, buyer, vnd(18_000)).await.unwrap().unwrap();
        assert_eq!((limit.amount, limit.units), (vnd(0), Some(0)));
        // Số dư là trần khi đơn lớn; dùng được đúng số đó.
        let limit = tender.limit(&tenant, buyer, vnd(1_000_000)).await.unwrap().unwrap();
        assert_eq!(limit.units, Some(100));
        let until = Timestamp::now() + SignedDuration::from_mins(30);
        let held = tender
            .hold(&tenant, "orders:max", buyer, &json!({"points": 100}), vnd(1_000_000), until)
            .await
            .unwrap();
        assert_eq!(held, limit.amount);
        // Đang giữ thì không tính.
        let limit = tender.limit(&tenant, buyer, vnd(1_000_000)).await.unwrap().unwrap();
        assert_eq!(limit.units, Some(0));
        // Tiền tệ khác chương trình: không báo.
        let usd = Money::new(10_000, Currency::parse("USD").unwrap());
        assert!(tender.limit(&tenant, buyer, usd).await.unwrap().is_none());
    })
    .await
    .unwrap();
}
