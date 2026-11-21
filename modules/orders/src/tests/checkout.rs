//! Đặt hàng: `Idempotency-Key` bắt buộc và chống trùng, giữ chỗ / trả chỗ, số đơn liền, thanh toán qua
//! `ledger`, hết hạn, thanh toán muộn, lượt đặt treo.

use std::sync::atomic::Ordering;

use autoxide_kernel::tenancy::{OrgId, ScopeExt};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use futures_util::future::join_all;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Call, PaymentSucceeded, Shop, drain, emit, run_job, shop};
use crate::entity::{checkout, order};

/// Giỏ của `call` ở cửa hàng A với `quantity` biến thể `variant`, đã có nơi giao; trả (id giỏ,
/// `cart_token` nếu vãng lai, `quote_token`).
pub(crate) async fn ready_cart(
    call: &Call<'_>,
    org: OrgId,
    variant: Uuid,
    quantity: i64,
) -> (String, Option<String>, String) {
    let (status, created) =
        call.send("POST", "/carts", Some(json!({"org_id": org.as_uuid()}))).await;
    assert!(status.is_success(), "{created}");
    let id = created["data"]["id"].as_str().unwrap().to_owned();
    let token = created["data"]["cart_token"].as_str().map(str::to_owned);
    let call = Call { app: call.app, who: call.who, cart_token: token.as_deref() };
    let (status, cart) = call
        .send("PUT", &format!("/carts/{id}/lines/{variant}"), Some(json!({"quantity": quantity})))
        .await;
    assert_eq!(status, StatusCode::OK, "{cart}");
    let delivery = json!({"address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1 Trang Tien"},
        "recipient_name": "An", "phone": "0901234567", "shipping_option": "standard"});
    let (status, cart) = call.send("PUT", &format!("/carts/{id}/delivery"), Some(delivery)).await;
    assert_eq!(status, StatusCode::OK, "{cart}");
    let quote = cart["data"]["quote_token"].as_str().unwrap().to_owned();
    (id, token, quote)
}

pub(crate) fn body(cart: &str, quote: &str, method: &str) -> Value {
    json!({"cart_id": cart, "quote_token": quote, "payment_method": method,
        "contact": {"name": "Nguyễn An", "phone": "0901234567"}})
}

/// `POST /checkout` với `key` (hay không khoá).
pub(crate) async fn checkout(
    call: &Call<'_>,
    key: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = call.app.post("/api/v1/orders/checkout").json(body);
    if let Some(who) = call.who {
        request = request.token(who);
    }
    if let Some(token) = call.cart_token {
        request = request.header("Cart-Token", token);
    }
    if let Some(key) = key {
        request = request.header("Idempotency-Key", key);
    }
    let answer = request.send().await.unwrap();
    let status = answer.status();
    (status, if answer.body().is_empty() { Value::Null } else { answer.json() })
}

async fn orders_count(shop: &Shop) -> u64 {
    use sea_orm::PaginatorTrait;
    order::Entity::find()
        .scoped(&shop.default_tenant())
        .count(shop.database().read())
        .await
        .unwrap()
}

#[tokio::test]
async fn checkout_requires_idempotency_key_and_replays_same_key() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 200_000);
        shop.inventory.stock(shop.tree.a, shirt, 5);
        let code = shop.promotions.add_code("GIAM10", 1000);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, _) = ready_cart(&buyer, shop.tree.a, shirt, 2).await;
        let (_, priced) = buyer
            .send("PUT", &format!("/carts/{cart}/codes"), Some(json!({"codes": ["GIAM10"]})))
            .await;
        let quote = priced["data"]["quote_token"].as_str().unwrap().to_owned();
        let (status, error) = checkout(&buyer, None, body(&cart, &quote, "cod")).await;
        assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
        assert_eq!(error["code"], "orders.idempotency_key_required");

        let (status, placed) = checkout(&buyer, Some("k-1"), body(&cart, &quote, "cod")).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        let order = &placed["data"]["orders"][0];
        assert_eq!(order["status"], "confirmed");
        assert_eq!(order["payment_status"], "cod_pending");
        assert_eq!(
            order["number"].as_str().unwrap(),
            format!("{}-000001", Utc::now().format("%Y"))
        );
        assert_eq!(super::minor(&order["totals"]["total"]), 400_000 - 40_000 + 30_000);
        assert_eq!(placed["data"]["payment_actions"][0]["kind"], "cod");
        let id: Uuid = order["id"].as_str().unwrap().parse().unwrap();
        assert_eq!(shop.inventory.hold_status(id), Some("held"));
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 3);
        drain(&shop).await;
        assert_eq!(shop.promotions.redemption_status(id), Some("confirmed"));
        let _ = code;

        // Gửi lại cùng khoá (kể cả khi giỏ đã thành đơn): đơn cũ, 200, không giữ thêm.
        let (status, again) = checkout(&buyer, Some("k-1"), body(&cart, &quote, "cod")).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again["data"]["orders"][0]["id"], placed["data"]["orders"][0]["id"]);
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 3);
        // Cùng khoá, nội dung khác: 422.
        let (status, error) = checkout(&buyer, Some("k-1"), body(&cart, &quote, "card")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["code"], "orders.idempotency_key_reused");
        // Giỏ đã thành đơn: khoá mới bị từ chối.
        let (status, error) = checkout(&buyer, Some("k-2"), body(&cart, &quote, "cod")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["code"], "orders.cart_closed");
        // Người mua thấy đơn của mình; người khác thì không.
        let (status, mine) = buyer.send("GET", &format!("/me/orders/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_eq!(mine["data"]["lines"][0]["quantity"], 2);
        let (status, _) =
            Call::as_user(&shop, "buyer2").send("GET", &format!("/me/orders/{id}"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, page) = Call::as_user(&shop, "staff_a").send("GET", "", None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn concurrent_checkouts_same_key_create_one_order() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 100);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        let attempts = (0..16).map(|_| checkout(&buyer, Some("same"), body(&cart, &quote, "cod")));
        let outcomes = join_all(attempts).await;
        let created = outcomes.iter().filter(|(status, _)| *status == StatusCode::CREATED).count();
        assert_eq!(created, 1, "{outcomes:?}");
        for (status, answer) in &outcomes {
            assert!(
                *status == StatusCode::CREATED
                    || *status == StatusCode::OK
                    || answer["code"] == "orders.checkout_in_progress",
                "{status} {answer}"
            );
        }
        assert_eq!(orders_count(&shop).await, 1);
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 99);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn concurrent_checkouts_other_keys_same_cart_create_one_order() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 100);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        // Mỗi lần một `Idempotency-Key` khác trên cùng giỏ. Bước kiểm giỏ còn mở chạy ngoài
        // transaction: nhiều lần từng cùng qua và ra nhiều đơn từ một giỏ (cả hai DB).
        let keys: Vec<String> = (0..8).map(|index| format!("other-{index}")).collect();
        let attempts =
            keys.iter().map(|key| checkout(&buyer, Some(key), body(&cart, &quote, "cod")));
        let outcomes = join_all(attempts).await;
        let created = outcomes.iter().filter(|(status, _)| *status == StatusCode::CREATED).count();
        assert_eq!(created, 1, "{outcomes:?}");
        for (status, answer) in &outcomes {
            assert!(
                *status == StatusCode::CREATED || answer["code"] == "orders.cart_closed",
                "{status} {answer}"
            );
        }
        assert_eq!(orders_count(&shop).await, 1);
        // Lượt thua đã trả chỗ giữ: chỉ một cái bị giữ.
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 99);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn last_unit_concurrent_checkouts() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 20);
        let mut carts = Vec::new();
        for _ in 0..20 {
            carts.push(ready_cart(&Call::as_guest(&shop, None), shop.tree.a, shirt, 1).await);
        }
        // 19 cái bán chỗ khác: còn đúng 1 cái cho 20 người.
        shop.inventory.stock(shop.tree.a, shirt, -19);
        let attempts = carts.iter().enumerate().map(|(index, (cart, token, quote))| {
            let shop = &shop;
            async move {
                let call = Call::as_guest(shop, token.as_deref());
                checkout(&call, Some(&format!("k{index}")), body(cart, quote, "cod")).await
            }
        });
        let outcomes = join_all(attempts).await;
        let created = outcomes.iter().filter(|(status, _)| *status == StatusCode::CREATED).count();
        assert_eq!(created, 1);
        let refused =
            outcomes.iter().filter(|(_, answer)| answer["code"] == "orders.out_of_stock").count();
        assert_eq!(refused, 19, "{outcomes:?}");
        assert_eq!(orders_count(&shop).await, 1);
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn promotion_hold_failure_releases_stock_hold_and_numbers_have_no_gaps() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 100);
        shop.promotions.add_code("GIAM10", 1000);
        let mut carts = Vec::new();
        for _ in 0..10 {
            let guest = Call::as_guest(&shop, None);
            let (cart, token, _) = ready_cart(&guest, shop.tree.a, shirt, 1).await;
            let call = Call::as_guest(&shop, token.as_deref());
            let (_, priced) = call
                .send("PUT", &format!("/carts/{cart}/codes"), Some(json!({"codes": ["GIAM10"]})))
                .await;
            carts.push((cart, token, priced["data"]["quote_token"].as_str().unwrap().to_owned()));
        }
        // Ba lượt giữ khuyến mại hỏng giữa chừng.
        shop.promotions.fail_reserve.store(3, Ordering::SeqCst);
        let attempts = carts.iter().enumerate().map(|(index, (cart, token, quote))| {
            let shop = &shop;
            async move {
                let call = Call::as_guest(shop, token.as_deref());
                checkout(&call, Some(&format!("k{index}")), body(cart, quote, "cod")).await
            }
        });
        let outcomes = join_all(attempts).await;
        let failed = outcomes
            .iter()
            .filter(|(_, answer)| answer["code"] == "orders.promotion_unavailable")
            .count();
        assert_eq!(failed, 3, "{outcomes:?}");
        // Chỗ giữ tồn của lượt hỏng đã trả: chỉ 7 cái bị giữ.
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 93);
        let numbers: Vec<String> = order::Entity::find()
            .scoped(&shop.default_tenant())
            .order_by_asc(order::Column::Number)
            .all(shop.database().read())
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.number)
            .collect();
        let year = Utc::now().format("%Y");
        let expected: Vec<String> = (1..=7).map(|n| format!("{year}-{n:06}")).collect();
        assert_eq!(numbers, expected);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quote_changed_between_quote_and_checkout() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 5);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        shop.catalog.set_price(shirt, 120_000);
        let (status, error) = checkout(&buyer, Some("k"), body(&cart, &quote, "cod")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["code"], "orders.quote_changed");
        assert_eq!(orders_count(&shop).await, 0);
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 5);
        // Giỏ không có hàng cần giao chưa chọn cách giao: COD từ chối, thiếu nơi giao → 422.
        let (other, _, _) =
            ready_cart(&Call::as_user(&shop, "buyer2"), shop.tree.a, shirt, 1).await;
        let buyer2 = Call::as_user(&shop, "buyer2");
        buyer2.send("PUT", &format!("/carts/{other}/delivery"), Some(json!({}))).await;
        let (_, cart) = buyer2.send("GET", &format!("/carts/{other}"), None).await;
        let quote = cart["data"]["quote_token"].as_str().unwrap();
        let (status, error) = checkout(&buyer2, Some("k"), body(&other, quote, "cod")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["code"], "orders.delivery_required");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn online_payment_confirms_once_and_unpaid_order_expires() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 5);
        shop.promotions.add_code("GIAM10", 1000);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, _) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        let (_, priced) = buyer.send("PUT", &format!("/carts/{cart}/codes"), Some(json!({"codes": ["GIAM10"]}))).await;
        let quote = priced["data"]["quote_token"].as_str().unwrap().to_owned();
        let (status, placed) = checkout(&buyer, Some("pay-1"), body(&cart, &quote, "card")).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        let order_json = &placed["data"]["orders"][0];
        assert_eq!(order_json["status"], "pending_payment");
        let action = &placed["data"]["payment_actions"][0];
        assert_eq!(action["kind"], "redirect", "{action}");
        assert!(action["redirect_url"].as_str().unwrap().starts_with("https://pay.example/"));
        let id: Uuid = order_json["id"].as_str().unwrap().parse().unwrap();
        // Màn chờ thanh toán đọc trạng thái công khai bằng token.
        let token = action["status_token"].as_str().unwrap();
        let (status, public) = Call::as_guest(&shop, None).send("GET", &format!("/public/payments/{token}"), None).await;
        assert_eq!(status, StatusCode::OK, "{public}");
        assert_eq!(public["data"]["status"], "pending");
        let (status, _) = Call::as_guest(&shop, None).send("GET", &format!("/public/payments/{id}.x"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // `ledger` báo đã thu, hai lần.
        let payment = shop.ledger.payments.lock().unwrap()[0].clone();
        let event = json!({"payment_id": payment.id, "source_module": "orders", "amount": payment.amount, "reference": payment.reference});
        emit(&shop, &PaymentSucceeded(event.clone())).await;
        emit(&shop, &PaymentSucceeded(event)).await;
        drain(&shop).await;
        let (_, mine) = buyer.send("GET", &format!("/me/orders/{id}"), None).await;
        assert_eq!(mine["data"]["status"], "confirmed");
        assert_eq!(mine["data"]["payment_status"], "paid");
        assert_eq!(super::minor(&mine["data"]["totals"]["paid"]), payment.amount.minor());
        assert_eq!(shop.promotions.redemption_status(id), Some("confirmed"));
        let (_, public) = Call::as_guest(&shop, None).send("GET", &format!("/public/payments/{token}"), None).await;
        assert_eq!(public["data"]["status"], "succeeded");

        // Đơn thứ hai không trả: quá hạn thì huỷ, trả lượt khuyến mại; trả muộn → hoàn tự động.
        let buyer2 = Call::as_user(&shop, "buyer2");
        let (cart, _, _) = ready_cart(&buyer2, shop.tree.a, shirt, 1).await;
        let (_, priced) = buyer2.send("PUT", &format!("/carts/{cart}/codes"), Some(json!({"codes": ["GIAM10"]}))).await;
        let quote = priced["data"]["quote_token"].as_str().unwrap().to_owned();
        let (_, placed) = checkout(&buyer2, Some("pay-2"), body(&cart, &quote, "bank_transfer")).await;
        let late: Uuid = placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap();
        assert_eq!(placed["data"]["payment_actions"][0]["kind"], "qr");
        order::Entity::update_many()
            .col_expr(order::Column::PaymentDueAt, Expr::value(Utc::now() - Duration::minutes(1)))
            .filter(order::Column::Id.eq(late))
            .exec(shop.database().write())
            .await
            .unwrap();
        run_job(&shop, crate::jobs::EXPIRE_UNPAID).await;
        let (_, cancelled) = buyer2.send("GET", &format!("/me/orders/{late}"), None).await;
        assert_eq!(cancelled["data"]["status"], "cancelled");
        assert_eq!(cancelled["data"]["cancel_reason"], "payment_expired");
        assert_eq!(shop.promotions.redemption_status(late), Some("released"));
        let payment = shop.ledger.payments.lock().unwrap()[1].clone();
        emit(&shop, &PaymentSucceeded(json!({"payment_id": payment.id, "source_module": "orders", "amount": payment.amount}))).await;
        drain(&shop).await;
        let (_, still) = buyer2.send("GET", &format!("/me/orders/{late}"), None).await;
        assert_eq!(still["data"]["status"], "cancelled");
        let refunds = shop.ledger.refunds.lock().unwrap().clone();
        assert_eq!(refunds, vec![(payment.id, payment.amount.minor(), "late_payment".to_owned())]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn crash_after_reserve_is_recovered_by_job() {
    TestDatabase::run(async |test| {
        use autoxide_inventory::api::{Inventory, ReserveLine, ReserveRequest};
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 5);
        let tenant = shop.default_tenant();
        // Máy chết ngay sau bước giữ tồn: lượt đặt kẹt ở `reserving`, đơn chưa có.
        let id = Uuid::now_v7();
        let lines = vec![ReserveLine::new(shirt.to_string(), shirt, 2)];
        let request = ReserveRequest::new(id, shop.tree.a, Utc::now() + Duration::hours(1), lines);
        shop.inventory.reserve(&tenant, &request).await.unwrap();
        let stuck = checkout::ActiveModel {
            id: sea_orm::Set(id),
            tenant_id: sea_orm::Set(tenant.id().as_uuid()),
            owner_key: sea_orm::Set("user:x".to_owned()),
            reference: sea_orm::Set("crash".to_owned()),
            fingerprint: sea_orm::Set("f".to_owned()),
            cart_id: sea_orm::Set(Uuid::now_v7()),
            state: sea_orm::Set("reserving".to_owned()),
            failure_code: sea_orm::Set(None),
            holds: sea_orm::Set("{}".to_owned()),
            created_at: sea_orm::Set(Utc::now() - Duration::hours(1)),
            updated_at: sea_orm::Set(Utc::now() - Duration::hours(1)),
        };
        sea_orm::ActiveModelTrait::insert(stuck, shop.database().write()).await.unwrap();
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 3);
        run_job(&shop, crate::jobs::RECOVER_CHECKOUTS).await;
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 5);
        let row = checkout::Entity::find()
            .scoped(&tenant)
            .one(shop.database().read())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn orders_api_finds_order_and_delivered_lines() {
    TestDatabase::run(async |test| {
        use crate::api::Orders;
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 5);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 2).await;
        let (_, placed) = checkout(&buyer, Some("k"), body(&cart, &quote, "cod")).await;
        let id: Uuid = placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap();
        let orders = shop.api::<dyn Orders>().unwrap();
        let tenant = shop.default_tenant();
        let found = orders.find(&tenant, id).await.unwrap().unwrap();
        assert_eq!(found.status, crate::api::OrderStatus::Confirmed);
        assert_eq!(found.lines[0].quantity, 2);
        assert_eq!(found.total.minor(), 230_000);
        assert!(orders.find(&tenant, Uuid::now_v7()).await.unwrap().is_none());
        // Chưa giao: chưa phải người đã nhận hàng.
        let buyer_id = Uuid::from_u128(101);
        assert!(orders.delivered_lines(&tenant, buyer_id, shirt).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}
