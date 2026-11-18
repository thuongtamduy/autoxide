//! Event của `orders` (module giả phát): xác nhận, huỷ, nhập lại hàng trả về — nhận lại nhiều lần
//! vẫn chỉ áp một lần.

use std::sync::Arc;

use autoxide_kernel::event::Outbox;
use autoxide_testkit::{TestApp, TestDatabase};
use chrono::{Duration, Utc};
use serde_json::json;
use std::sync::Mutex;
use uuid::Uuid;

use super::{Emitter, assert_consistent, drain, emit, level, raw_event, shop_with, stock_in};
use crate::api::{Inventory, ReservationStatus, ReserveLine, ReserveRequest};

raw_event!(OrderConfirmed, "orders.order_confirmed");
raw_event!(OrderCancelled, "orders.order_cancelled");
raw_event!(ReturnReceived, "orders.return_received");

const EVENTS: [&str; 3] =
    ["orders.order_confirmed", "orders.order_cancelled", "orders.return_received"];

async fn status(app: &TestApp, order: Uuid) -> serde_json::Value {
    let (_, found) =
        super::call(app, "GET", &format!("/reservations?order_id={order}"), "boss", None).await;
    found["data"][0]["status"].clone()
}

async fn twice<E: autoxide_kernel::event::Event>(
    app: &TestApp,
    outbox: &Arc<Mutex<Option<Outbox>>>,
    event: &E,
) {
    emit(app, outbox, None, event).await;
    emit(app, outbox, None, event).await;
    drain(app).await;
}

#[tokio::test]
async fn nhan_lai_event_xac_nhan_huy_va_tra_hang() {
    TestDatabase::run(async |test| {
        let (orders, outbox) = Emitter::new("orders", &EVENTS);
        let shop = shop_with(test, |builder, _| builder.module(orders)).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 10, None).await;
        let api = shop.api::<dyn Inventory>().unwrap();
        let tenant = shop.default_tenant();
        let reserve = |order: Uuid| {
            ReserveRequest::new(order, shop.tree.a, Utc::now() + Duration::minutes(30), vec![ReserveLine::new("1", item, 3)])
        };
        let (kept, dropped) = (Uuid::now_v7(), Uuid::now_v7());
        api.reserve(&tenant, &reserve(kept)).await.unwrap();
        api.reserve(&tenant, &reserve(dropped)).await.unwrap();

        // Event xác nhận đến trước lời gọi trực tiếp, rồi đến lại: vẫn một lần xác nhận.
        twice(&shop, &outbox, &OrderConfirmed(json!({"after": {"order_id": kept}}))).await;
        assert_eq!(status(&shop, kept).await, "confirmed");
        assert_eq!(api.confirm(&tenant, kept).await.unwrap().status, ReservationStatus::Confirmed);
        // Huỷ hai lần: cộng lại 3 đúng một lần.
        twice(&shop, &outbox, &OrderCancelled(json!({"after": {"order_id": dropped}}))).await;
        assert_eq!(status(&shop, dropped).await, "released");
        assert_eq!(level(&shop, shop.tree.a, item).await["available"], 7);
        // Huỷ đơn không có chỗ giữ và xác nhận đơn không có chỗ giữ: bỏ qua, không lỗi.
        twice(&shop, &outbox, &OrderCancelled(json!({"after": {"order_id": Uuid::now_v7()}}))).await;
        twice(&shop, &outbox, &OrderConfirmed(json!({"after": {"order_id": Uuid::now_v7()}}))).await;
        // Trả hàng: 2 bán lại được, 1 hỏng; giao hai lần chỉ nhập một lần.
        let returned = ReturnReceived(json!({"after": {
            "return_id": Uuid::now_v7(), "order_id": kept, "warehouse_org_id": shop.tree.a.as_uuid(),
            "lines": [
                {"variant_id": item, "quantity": 2, "sellable": true},
                {"variant_id": item, "quantity": 1, "sellable": false},
            ],
        }}));
        twice(&shop, &outbox, &returned).await;
        let found = level(&shop, shop.tree.a, item).await;
        assert_eq!((found["on_hand"].as_i64(), found["available"].as_i64()), (Some(12), Some(9)));
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}
