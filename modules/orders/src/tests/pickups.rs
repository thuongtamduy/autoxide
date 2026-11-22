//! Nhận tại cửa hàng: sẵn sàng → mã nhận chỉ người mua thấy, giao bằng mã hay người nhận hộ, nhắc
//! trước hạn một lần, hết hạn thì huỷ (mặc định) hay chờ tiếp theo chính sách.

use super::CatalogExt as _;
use autoxide_kernel::tenancy::ScopeExt;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::checkout;
use super::lifecycle::{count_events, status_of};
use super::{Call, Shop, run_job, shop};
use crate::entity::pickup;

/// Đơn COD nhận tại cửa hàng A của `buyer1`, có người nhận hộ; trả ID.
async fn store_order(shop: &Shop) -> Uuid {
    let shirt = shop.catalog.add(shop.tree.a, 100_000);
    shop.inventory.stock(shop.tree.a, shirt, 10);
    let buyer = Call::as_user(shop, "buyer1");
    let (_, created) =
        buyer.send("POST", "/carts", Some(json!({"org_id": shop.tree.a.as_uuid()}))).await;
    let cart = created["data"]["id"].as_str().unwrap().to_owned();
    buyer.send("PUT", &format!("/carts/{cart}/lines/{shirt}"), Some(json!({"quantity": 1}))).await;
    let delivery = json!({"address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1 Trang Tien"},
        "recipient_name": "An", "phone": "0901234567", "shipping_option": "store"});
    let (status, seen) =
        buyer.send("PUT", &format!("/carts/{cart}/delivery"), Some(delivery)).await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    let quote = seen["data"]["quote_token"].as_str().unwrap();
    let body = json!({"cart_id": cart, "quote_token": quote, "payment_method": "cod",
        "contact": {"name": "Nguyễn An", "phone": "0901234567", "proxy_name": "Trần Bình", "proxy_phone": "0912345678"}});
    let (status, placed) = checkout(&buyer, Some(&Uuid::now_v7().to_string()), body).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap()
}

async fn boss(shop: &Shop, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    Call::as_user(shop, "boss").send("POST", uri, body).await
}

async fn age(shop: &Shop, id: Uuid, until: Timestamp) {
    pickup::Entity::update_many()
        .col_expr(pickup::Column::HoldUntil, Expr::value(until))
        .filter(pickup::Column::OrderId.eq(id))
        .scoped(&shop.default_tenant())
        .exec(shop.database().write())
        .await
        .unwrap();
}

#[tokio::test]
async fn ready_order_is_handed_over_by_code_or_proxy() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let id = store_order(&shop).await;
        // Chưa sẵn sàng thì chưa giao được.
        let (status, error) =
            boss(&shop, &format!("/{id}/pickup/handover"), Some(json!({"code": "123456"}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        assert_eq!(error["code"], "orders.pickup_not_ready");
        let (status, ready) = boss(&shop, &format!("/{id}/pickup/ready"), None).await;
        assert_eq!(status, StatusCode::OK, "{ready}");
        // Giữ 2 ngày (của phương thức nhận tại cửa hàng); đơn đã được nhận (`preparing`).
        assert_eq!(status_of(&shop, id).await, "preparing");
        let (_, again) = boss(&shop, &format!("/{id}/pickup/ready"), None).await;
        assert_eq!(again["data"]["hold_until"], ready["data"]["hold_until"]);
        assert_eq!(count_events(&shop, "orders.pickup_ready").await, 1);
        // Mã chỉ người mua thấy.
        let (_, mine) =
            Call::as_user(&shop, "buyer1").send("GET", &format!("/me/orders/{id}"), None).await;
        let code = mine["data"]["pickup"]["code"].as_str().unwrap().to_owned();
        assert_eq!(code.len(), 6);
        let (_, staff) = Call::as_user(&shop, "boss").send("GET", &format!("/{id}"), None).await;
        assert!(staff["data"]["pickup"].get("code").is_none(), "{staff}");
        let (_, listed) =
            Call::as_user(&shop, "boss").send("GET", "?pickup_status=ready", None).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);
        // Sai mã, sai người nhận hộ: 422.
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let (status, error) =
            boss(&shop, &format!("/{id}/pickup/handover"), Some(json!({"code": wrong}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["code"], "orders.pickup_code_invalid");
        let (status, _) = boss(
            &shop,
            &format!("/{id}/pickup/handover"),
            Some(json!({"proxy_name": "Người Lạ", "proxy_phone": "0912345678"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        // Người nhận hộ đúng tên và số (khác định dạng): giao.
        let (status, delivered) = boss(
            &shop,
            &format!("/{id}/pickup/handover"),
            Some(json!({"proxy_name": "trần  bình", "proxy_phone": "+84912345678"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{delivered}");
        assert_eq!(delivered["data"]["status"], "delivered");
        assert_eq!(delivered["data"]["pickup"]["status"], "picked_up");

        // Đơn thứ hai: giao bằng mã.
        let second = store_order(&shop).await;
        boss(&shop, &format!("/{second}/pickup/ready"), None).await;
        let (_, mine) =
            Call::as_user(&shop, "buyer1").send("GET", &format!("/me/orders/{second}"), None).await;
        let code = mine["data"]["pickup"]["code"].as_str().unwrap();
        let (status, delivered) =
            boss(&shop, &format!("/{second}/pickup/handover"), Some(json!({"code": code}))).await;
        assert_eq!(status, StatusCode::OK, "{delivered}");
        assert_eq!(delivered["data"]["status"], "delivered");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn pickup_reminds_once_then_expires_per_policy() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let id = store_order(&shop).await;
        boss(&shop, &format!("/{id}/pickup/ready"), None).await;
        // Còn dưới 24 giờ: nhắc đúng một lần.
        age(&shop, id, Timestamp::now() + SignedDuration::from_hours(5)).await;
        run_job(&shop, crate::jobs::EXPIRE_PICKUPS).await;
        run_job(&shop, crate::jobs::EXPIRE_PICKUPS).await;
        assert_eq!(count_events(&shop, "orders.pickup_reminder").await, 1);
        // Quá hạn, mặc định `cancel_refund`: huỷ.
        age(&shop, id, Timestamp::now() - SignedDuration::from_hours(1)).await;
        run_job(&shop, crate::jobs::EXPIRE_PICKUPS).await;
        assert_eq!(status_of(&shop, id).await, "cancelled");
        assert_eq!(count_events(&shop, "orders.pickup_expired").await, 1);

        // `keep_waiting`: đánh dấu hết hạn, vẫn giao được.
        shop.orgs.set(shop.tree.a, "orders.pickup_expired_action", json!("keep_waiting"));
        let second = store_order(&shop).await;
        boss(&shop, &format!("/{second}/pickup/ready"), None).await;
        age(&shop, second, Timestamp::now() - SignedDuration::from_hours(1)).await;
        run_job(&shop, crate::jobs::EXPIRE_PICKUPS).await;
        assert_eq!(status_of(&shop, second).await, "preparing");
        let (_, mine) =
            Call::as_user(&shop, "buyer1").send("GET", &format!("/me/orders/{second}"), None).await;
        assert_eq!(mine["data"]["pickup"]["status"], "expired");
        let code = mine["data"]["pickup"]["code"].as_str().unwrap();
        let (status, delivered) =
            boss(&shop, &format!("/{second}/pickup/handover"), Some(json!({"code": code}))).await;
        assert_eq!(status, StatusCode::OK, "{delivered}");
    })
    .await
    .unwrap();
}
