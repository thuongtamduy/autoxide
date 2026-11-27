//! Xuất kho theo event giao hàng (`delivery`): dòng tường minh theo thứ tự nào cũng đúng, phần đặt trước đã
//! huỷ không xuất, hàng theo serial chờ số serial, lô quá hạn vẫn xuất được — event tới hai lần vẫn một lần.

use super::CatalogExt as _;
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::{Arc, Mutex};

use autoxide_kernel::event::Outbox;
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Emitter, assert_consistent, call, drain, emit, level, raw_event, shop_with, stock_in};
use crate::api::{Inventory, ReserveLine, ReserveRequest};

raw_event!(FulfillmentDelivered, "orders.fulfillment_delivered");
raw_event!(OrderDelivered, "orders.order_delivered");
raw_event!(SerialsRecorded, "orders.line_serials_recorded");
raw_event!(PreorderCancelled, "orders.preorder_cancelled");

const EVENTS: [&str; 4] = [
    "orders.fulfillment_delivered",
    "orders.order_delivered",
    "orders.line_serials_recorded",
    "orders.preorder_cancelled",
];

async fn twice<E: autoxide_kernel::event::Event>(
    app: &TestApp,
    outbox: &Arc<Mutex<Option<Outbox>>>,
    event: &E,
) {
    emit(app, outbox, None, event).await;
    emit(app, outbox, None, event).await;
    drain(app).await;
}

async fn status(app: &TestApp, order: Uuid) -> Value {
    let (_, found) =
        call(app, "GET", &format!("/reservations?order_id={order}"), "boss", None).await;
    found["data"][0]["status"].clone()
}

/// (`on_hand`, `reserved`) của biến thể ở kho A.
async fn stock(app: &super::Shop, variant: Uuid) -> (i64, i64) {
    let found = level(app, app.tree.a, variant).await;
    (found["on_hand"].as_i64().unwrap(), found["reserved"].as_i64().unwrap())
}

async fn reserve(shop: &super::Shop, order: Uuid, lines: Vec<ReserveLine>) {
    let api = shop.api::<dyn Inventory>().unwrap();
    let tenant = shop.default_tenant();
    let request = ReserveRequest::new(
        order,
        shop.tree.a,
        Timestamp::now() + SignedDuration::from_mins(30),
        lines,
    );
    api.reserve(&tenant, &request).await.unwrap();
    api.confirm(&tenant, order).await.unwrap();
}

/// `orders.order_delivered` tới TRƯỚC lần giao lẻ: mỗi event xuất đúng dòng của nó; tới lại không xuất thêm.
/// Event cũ (không có `delivered_lines`) tới sau khi đã xuất hết: xong, không lỗi.
#[tokio::test]
async fn giao_hang_dong_tuong_minh_thu_tu_nguoc() {
    TestDatabase::run(async |test| {
        let (orders, outbox) = Emitter::new("orders", &EVENTS);
        let shop = shop_with(test, |builder, _| builder.module(orders)).await;
        let item = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 10, None).await;
        let order = Uuid::now_v7();
        reserve(&shop, order, vec![ReserveLine::new("1", item, 3)]).await;

        let all = OrderDelivered(json!({"after": {
            "order_id": order, "lines": [{"line_key": "1"}],
            "delivered_lines": [{"line_key": "1", "quantity": 2, "serials": []}],
        }}));
        twice(&shop, &outbox, &all).await;
        assert_eq!(stock(&shop, item).await, (8, 1));
        assert_eq!(status(&shop, order).await, "confirmed");
        let part = FulfillmentDelivered(json!({"after": {
            "order_id": order, "fulfillment_id": Uuid::now_v7(),
            "lines": [{"line_key": "1", "quantity": 1, "serials": []}],
        }}));
        twice(&shop, &outbox, &part).await;
        assert_eq!(stock(&shop, item).await, (7, 0));
        assert_eq!(status(&shop, order).await, "fulfilled");
        let old =
            OrderDelivered(json!({"after": {"order_id": order, "lines": [{"line_key": "1"}]}}));
        twice(&shop, &outbox, &old).await;
        assert_eq!(stock(&shop, item).await, (7, 0));
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

/// Phần đặt trước đã huỷ không bị xuất: `delivered_lines` nêu phần còn lại, có nêu thừa cũng chỉ xuất phần
/// còn giữ.
#[tokio::test]
async fn giao_hang_sau_khi_huy_phan_dat_truoc() {
    TestDatabase::run(async |test| {
        let (orders, outbox) = Emitter::new("orders", &EVENTS);
        let shop = shop_with(test, |builder, _| builder.module(orders)).await;
        let ready = shop.catalog.item();
        let pre = shop.catalog.add(|variant| {
            variant.sale_mode = autoxide_catalog::api::SaleMode::Preorder;
            variant.preorder_limit = Some(5);
        });
        stock_in(&shop, shop.tree.a, ready, 10, None).await;
        stock_in(&shop, shop.tree.a, pre, 4, None).await;
        let order = Uuid::now_v7();
        reserve(&shop, order, vec![ReserveLine::new("ready", ready, 2), ReserveLine::new("pre", pre, 4)])
            .await;
        let cancelled = PreorderCancelled(json!({"after": {
            "order_id": order, "reason": "buyer_cancelled", "amount": "0",
            "lines": [{"line_id": Uuid::now_v7(), "line_key": "pre", "variant_id": pre, "quantity": 3}],
        }}));
        // Mỗi lần phát là một event mới (huỷ thêm); chỉ phát một lần, rồi giao lại outbox.
        emit(&shop, &outbox, None, &cancelled).await;
        drain(&shop).await;
        // `pre` nêu thừa (4 thay vì 1): chỉ xuất 1 đơn vị còn giữ.
        let delivered = OrderDelivered(json!({"after": {
            "order_id": order, "lines": [{"line_key": "ready"}, {"line_key": "pre"}],
            "delivered_lines": [
                {"line_key": "ready", "quantity": 2, "serials": []},
                {"line_key": "pre", "quantity": 4, "serials": []},
            ],
        }}));
        twice(&shop, &outbox, &delivered).await;
        assert_eq!(stock(&shop, ready).await, (8, 0));
        assert_eq!(stock(&shop, pre).await, (3, 0));
        assert_eq!(status(&shop, order).await, "fulfilled");
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

/// Đơn trộn hàng thường và hàng theo serial: giao khi chưa ghi serial thì hàng thường xuất, hàng serial giữ
/// nguyên; ghi serial (từng phần) thì kho xuất tiếp đúng các số đó.
#[tokio::test]
async fn giao_hang_serial_cho_so_serial() {
    TestDatabase::run(async |test| {
        let (orders, outbox) = Emitter::new("orders", &EVENTS);
        let shop = shop_with(test, |builder, _| builder.module(orders)).await;
        let item = shop.catalog.item();
        let phone = shop.catalog.add(|variant| variant.serial_tracked = true);
        stock_in(&shop, shop.tree.a, item, 5, None).await;
        let body = json!({"org_id": shop.tree.a.as_uuid(), "reference": "in-phones", "lines": [
            {"variant_id": phone, "quantity": 3, "reason_code": "found", "serials": ["S1", "S2", "S3"]},
        ]});
        let (_, made) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
        assert!(made["data"].is_object(), "{made}");
        let order = Uuid::now_v7();
        reserve(&shop, order, vec![ReserveLine::new("1", item, 2), ReserveLine::new("2", phone, 2)])
            .await;

        let delivered = OrderDelivered(json!({"after": {
            "order_id": order, "lines": [{"line_key": "1"}, {"line_key": "2"}],
            "delivered_lines": [
                {"line_key": "1", "quantity": 2, "serials": []},
                {"line_key": "2", "quantity": 2, "serials": []},
            ],
        }}));
        twice(&shop, &outbox, &delivered).await;
        assert_eq!(stock(&shop, item).await, (3, 0));
        assert_eq!(stock(&shop, phone).await, (3, 2));
        assert_eq!(status(&shop, order).await, "confirmed");

        let recorded = |serials: Value| {
            SerialsRecorded(json!({"after": {
                "order_id": order, "line_id": Uuid::nil(), "line_key": "2",
                "delivered_quantity": 2, "serials": serials,
            }}))
        };
        twice(&shop, &outbox, &recorded(json!(["S2"]))).await;
        assert_eq!(stock(&shop, phone).await, (2, 1));
        // Số chưa có trong kho (gõ sai) bị bỏ qua; S3 xuất.
        twice(&shop, &outbox, &recorded(json!(["S2", "S9", "S3"]))).await;
        assert_eq!(stock(&shop, phone).await, (1, 0));
        assert_eq!(status(&shop, order).await, "fulfilled");
        // S1 (không nêu) vẫn trong kho: đơn khác bán được S1, không bán được S2 đã xuất.
        let api = shop.api::<dyn Inventory>().unwrap();
        let tenant = shop.default_tenant();
        let other = Uuid::now_v7();
        reserve(&shop, other, vec![ReserveLine::new("x", phone, 1)]).await;
        let sell = |serial: &str| {
            crate::api::FulfilRequest::new(
                other,
                format!("sell-{serial}"),
                vec![crate::api::FulfilLine::new("x", 1).serials(vec![serial.to_owned()])],
            )
        };
        assert!(api.fulfil(&tenant, &sell("S2")).await.is_err());
        api.fulfil(&tenant, &sell("S1")).await.unwrap();
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

/// Hàng đã giao từ lô quá hạn (không còn lô còn hạn): event giao hàng vẫn xuất (ghi cảnh báo trong log).
#[tokio::test]
async fn giao_hang_xuat_ca_lo_qua_han() {
    TestDatabase::run(async |test| {
        let (orders, outbox) = Emitter::new("orders", &EVENTS);
        let shop = shop_with(test, |builder, _| builder.module(orders)).await;
        // Lịch trừ lô quá hạn (mỗi ngày) chạy ở lần `drain` đầu: cho nó chạy trước khi có lô quá hạn.
        drain(&shop).await;
        let milk = shop.catalog.add(|variant| variant.lot_tracked = true);
        let yesterday = Timestamp::now().date_utc().checked_add_days(-1).unwrap().to_string();
        let tomorrow = Timestamp::now().date_utc().checked_add_days(1).unwrap().to_string();
        let body = json!({"org_id": shop.tree.a.as_uuid(), "reference": "in-milk", "lines": [
            {"variant_id": milk, "quantity": 2, "reason_code": "found", "lot_code": "OLD", "expires_on": yesterday},
            {"variant_id": milk, "quantity": 1, "reason_code": "found", "lot_code": "NEW", "expires_on": tomorrow},
        ]});
        let (_, made) = call(&shop, "POST", "/adjustments", "boss", Some(body)).await;
        assert!(made["data"].is_object(), "{made}");
        let order = Uuid::now_v7();
        reserve(&shop, order, vec![ReserveLine::new("1", milk, 2)]).await;
        let delivered = OrderDelivered(json!({"after": {
            "order_id": order, "lines": [{"line_key": "1"}],
            "delivered_lines": [{"line_key": "1", "quantity": 2, "serials": []}],
        }}));
        twice(&shop, &outbox, &delivered).await;
        assert_eq!(stock(&shop, milk).await, (1, 0));
        assert_eq!(status(&shop, order).await, "fulfilled");
        // Lô còn hạn lấy trước, phần thiếu lấy từ lô quá hạn.
        let (_, lots) = call(&shop, "GET", &format!("/lots?variant_id={milk}"), "boss", None).await;
        let left: Vec<(&str, i64)> = lots["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|lot| (lot["lot_code"].as_str().unwrap(), lot["on_hand"].as_i64().unwrap()))
            .collect();
        assert!(left.contains(&("OLD", 1)) && left.contains(&("NEW", 0)), "{lots}");
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

/// Nhiều tiến trình giao event cùng lúc (D-054): hai lần giao lẻ và `orders.order_delivered`, mỗi event
/// phát hai lần (hãng báo lại), bốn dispatcher chạy song song — kho xuất đúng một lần mỗi phần, không thừa,
/// không thiếu; chỗ giữ `fulfilled`, sổ khớp tồn.
#[tokio::test]
async fn giao_hang_dong_thoi_xuat_dung_mot_lan() {
    TestDatabase::run(async |test| {
        let (orders, outbox) = Emitter::new("orders", &EVENTS);
        let shop = shop_with(test, |builder, _| builder.module(orders)).await;
        // Lịch chạy mỗi ngày của module chạy trước, để lượt song song bên dưới chỉ còn event giao hàng.
        drain(&shop).await;
        let item = shop.catalog.item();
        let other = shop.catalog.item();
        stock_in(&shop, shop.tree.a, item, 10, None).await;
        stock_in(&shop, shop.tree.a, other, 5, None).await;
        let order = Uuid::now_v7();
        reserve(&shop, order, vec![ReserveLine::new("1", item, 6), ReserveLine::new("2", other, 3)])
            .await;

        let first = FulfillmentDelivered(json!({"after": {
            "order_id": order, "fulfillment_id": Uuid::now_v7(),
            "lines": [{"line_key": "1", "quantity": 2, "serials": []}],
        }}));
        let second = FulfillmentDelivered(json!({"after": {
            "order_id": order, "fulfillment_id": Uuid::now_v7(),
            "lines": [{"line_key": "1", "quantity": 1, "serials": []}, {"line_key": "2", "quantity": 1, "serials": []}],
        }}));
        let rest = OrderDelivered(json!({"after": {
            "order_id": order, "lines": [{"line_key": "1"}, {"line_key": "2"}],
            "delivered_lines": [
                {"line_key": "1", "quantity": 3, "serials": []},
                {"line_key": "2", "quantity": 2, "serials": []},
            ],
        }}));
        for _ in 0..2 {
            emit(&shop, &outbox, None, &first).await;
            emit(&shop, &outbox, None, &rest).await;
            emit(&shop, &outbox, None, &second).await;
        }
        let dispatchers: Vec<_> = (0..4).map(|_| shop.modules().dispatcher().unwrap()).collect();
        // Mỗi dispatcher nhận (claim) event trước khi giao: các event khác nhau chạy song song.
        for outcome in futures_util::future::join_all(
            dispatchers.iter().map(|dispatcher| dispatcher.run_once()),
        )
        .await
        {
            outcome.unwrap();
        }
        drain(&shop).await;

        assert_eq!(stock(&shop, item).await, (4, 0));
        assert_eq!(stock(&shop, other).await, (2, 0));
        assert_eq!(status(&shop, order).await, "fulfilled");
        for (variant, total) in [(item, -6), (other, -3)] {
            let uri = format!("/movements?variant_id={variant}&kind=sale");
            let (_, sales) = call(&shop, "GET", &uri, "boss", None).await;
            let sold: i64 =
                sales["data"].as_array().unwrap().iter().filter_map(|m| m["quantity"].as_i64()).sum();
            assert_eq!(sold, total, "{sales}");
        }
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}
