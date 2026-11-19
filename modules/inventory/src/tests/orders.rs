//! Event của `orders` (module giả phát): xác nhận, huỷ, nhập lại hàng trả về, huỷ riêng phần đặt
//! trước — nhận lại nhiều lần vẫn chỉ áp một lần.

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

raw_event!(PreorderCancelled, "orders.preorder_cancelled");

/// Giao lại MỌI event của bảng outbox `orders_events` (như khi bộ phát giao lại sau sự cố): cùng ID event.
async fn redeliver(app: &TestApp) {
    use sea_orm::{ConnectionTrait, Statement};
    let write = app.database().write();
    let backend = write.get_database_backend();
    write
        .execute_raw(Statement::from_string(
            backend,
            "UPDATE orders_events SET delivered_at = NULL, claimed_until = NULL",
        ))
        .await
        .unwrap();
    drain(app).await;
    let row = write
        .query_one_raw(Statement::from_string(
            backend,
            "SELECT COUNT(*) AS n FROM orders_events WHERE delivered_at IS NULL",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "n").unwrap(), 0, "every event was delivered again");
}

#[tokio::test]
async fn huy_rieng_phan_dat_truoc_tra_suat_dung_mot_lan() {
    TestDatabase::run(async |test| {
        let (orders, outbox) = Emitter::new("orders", &["orders.preorder_cancelled"]);
        let shop = shop_with(test, |builder, _| builder.module(orders)).await;
        let ready = shop.catalog.item();
        let pre = shop.catalog.add(|variant| {
            variant.sale_mode = autoxide_catalog::api::SaleMode::Preorder;
            variant.preorder_limit = Some(5);
        });
        stock_in(&shop, shop.tree.a, ready, 10, None).await;
        let api = shop.api::<dyn Inventory>().unwrap();
        let tenant = shop.default_tenant();
        let order = Uuid::now_v7();
        let request = ReserveRequest::new(
            order,
            shop.tree.a,
            Utc::now() + Duration::minutes(30),
            vec![ReserveLine::new("ready", ready, 2), ReserveLine::new("pre", pre, 4)],
        );
        api.reserve(&tenant, &request).await.unwrap();
        api.confirm(&tenant, order).await.unwrap();
        // Suất còn 1: đặt 2 nữa thì hết suất.
        let more = |quantity| {
            ReserveRequest::new(
                Uuid::now_v7(),
                shop.tree.a,
                Utc::now() + Duration::minutes(30),
                vec![ReserveLine::new("pre", pre, quantity)],
            )
        };
        assert_eq!(
            api.reserve(&tenant, &more(2)).await.unwrap_err().code(),
            "inventory.preorder_sold_out"
        );

        // Huỷ 3 trong 4 đơn vị đặt trước; event giao lại (cùng ID) không trả thêm.
        let cancelled = PreorderCancelled(json!({"after": {
            "order_id": order, "reason": "preorder_stopped", "amount": "0",
            "lines": [{"line_id": Uuid::now_v7(), "line_key": "pre", "variant_id": pre, "quantity": 3}],
        }}));
        emit(&shop, &outbox, None, &cancelled).await;
        drain(&shop).await;
        redeliver(&shop).await;
        assert_eq!(level(&shop, shop.tree.a, pre).await["available"], -1);
        // Đơn vẫn còn hiệu lực, phần hàng sẵn không đổi.
        assert_eq!(status(&shop, order).await, "confirmed");
        assert_eq!(level(&shop, shop.tree.a, ready).await["available"], 8);
        // Suất trả đúng 3: đặt thêm 4 được, thêm 1 nữa thì hết.
        api.reserve(&tenant, &more(4)).await.unwrap();
        assert_eq!(
            api.reserve(&tenant, &more(1)).await.unwrap_err().code(),
            "inventory.preorder_sold_out"
        );
        // Lần huỷ thứ hai (event khác) huỷ phần còn lại; huỷ quá phần còn lại thì chỉ trả phần còn.
        let rest = PreorderCancelled(json!({"after": {
            "order_id": order, "reason": "buyer_cancelled", "amount": "0",
            "lines": [{"line_id": Uuid::now_v7(), "line_key": "pre", "variant_id": pre, "quantity": 5}],
        }}));
        emit(&shop, &outbox, None, &rest).await;
        drain(&shop).await;
        assert_eq!(level(&shop, shop.tree.a, pre).await["available"], -4);
        // Huỷ cả đơn sau đó chỉ trả phần hàng sẵn còn lại.
        api.release(&tenant, order, crate::api::ReleaseReason::Cancelled).await.unwrap();
        assert_eq!(level(&shop, shop.tree.a, ready).await["available"], 10);
        assert_eq!(level(&shop, shop.tree.a, pre).await["available"], -4);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}
