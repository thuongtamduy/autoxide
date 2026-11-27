//! Hàng đặt trước (D-204): đồng ý điều kiện trước khi đặt, đơn trộn hàng sẵn và đặt trước giao
//! hai lần (COD thu theo từng lần), dời ngày thì báo và cho huỷ miễn phí, ngừng bán đặt trước thì
//! huỷ phần chưa có hàng và hoàn tiền tự động.

use super::CatalogExt as _;
use autoxide_catalog::api::SaleMode;
use autoxide_kernel::tenancy::ScopeExt;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::{body, checkout};
use super::lifecycle::{count_events, status_of};
use super::{
    Call, CodCollected, PaymentSucceeded, PreorderAllocated, PreorderDateChanged,
    ShipmentDelivered, ShipmentStatusChanged, Shop, VariantUpdated, drain, emit, emit_from,
    keyed_shipment_event, minor, shop,
};
use crate::entity::line;

/// Biến thể đặt trước của cửa hàng A, giá `price`, hàng về sau 10 ngày.
pub(super) fn preorder_item(shop: &Shop, price: i64) -> Uuid {
    let id = shop.catalog.add_with(shop.tree.a, price, |variant| {
        variant.sale_mode = SaleMode::Preorder;
        variant.preorder_available_on =
            Some((Timestamp::now() + SignedDuration::from_hours(24 * 10)).date_utc());
    });
    shop.inventory.stock(shop.tree.a, id, 10);
    id
}

/// Giỏ của `buyer1` ở cửa hàng A với `items` (biến thể, số lượng); trả (giỏ, mã báo giá).
pub(super) async fn cart_of(shop: &Shop, items: &[(Uuid, i64)]) -> (String, String) {
    let buyer = Call::as_user(shop, "buyer1");
    let (_, created) =
        buyer.send("POST", "/carts", Some(json!({"org_id": shop.tree.a.as_uuid()}))).await;
    let cart = created["data"]["id"].as_str().unwrap().to_owned();
    for (variant, quantity) in items {
        let (status, seen) = buyer
            .send(
                "PUT",
                &format!("/carts/{cart}/lines/{variant}"),
                Some(json!({"quantity": quantity})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{seen}");
    }
    let delivery = json!({"address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "1 Trang Tien"},
        "recipient_name": "An", "phone": "0901234567", "shipping_option": "standard"});
    let (_, seen) = buyer.send("PUT", &format!("/carts/{cart}/delivery"), Some(delivery)).await;
    // Báo giá cho thấy ngày giao dự kiến của hàng đặt trước trước khi đặt.
    let lines = seen["data"]["lines"].as_array().unwrap();
    for line in lines.iter().filter(|line| !line["preorder"].is_null()) {
        assert!(line["preorder"]["available_on"].is_string(), "{line}");
    }
    (cart, seen["data"]["quote_token"].as_str().unwrap().to_owned())
}

async fn place(shop: &Shop, items: &[(Uuid, i64)], method: &str) -> Uuid {
    let (cart, quote) = cart_of(shop, items).await;
    let mut request = body(&cart, &quote, method);
    request["accept_preorder_terms"] = json!(true);
    let buyer = Call::as_user(shop, "buyer1");
    let (status, placed) = checkout(&buyer, Some(&Uuid::now_v7().to_string()), request).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    let id = placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap();
    if method == "card" {
        let payment = shop.ledger.last_payment().unwrap();
        emit(shop, &PaymentSucceeded(json!({"payment_id": payment.id, "source_module": "orders", "amount": payment.amount}))).await;
        drain(shop).await;
    }
    id
}

pub(super) async fn ship(shop: &Shop, id: Uuid) -> (StatusCode, Value) {
    Call::as_user(shop, "boss").send("POST", &format!("/{id}/shipments"), Some(json!({}))).await
}

async fn line_of(shop: &Shop, order: Uuid, variant: Uuid) -> line::Model {
    line::Entity::find()
        .filter(line::Column::OrderId.eq(order))
        .filter(line::Column::VariantId.eq(variant))
        .scoped(&shop.default_tenant())
        .one(shop.database().read())
        .await
        .unwrap()
        .unwrap()
}

async fn deliver(shop: &Shop, id: Uuid, key: &str, cod: Option<i64>) {
    let event = keyed_shipment_event(id, key, "delivered", None);
    emit_from(shop, &shop.shipping_outbox, &ShipmentDelivered(event)).await;
    if cod.is_some() {
        let collected = keyed_shipment_event(id, key, "delivered", cod);
        emit_from(shop, &shop.shipping_outbox, &CodCollected(collected.clone())).await;
        // Hãng báo lại: không cộng hai lần.
        emit_from(shop, &shop.shipping_outbox, &CodCollected(collected)).await;
    }
    drain(shop).await;
}

#[tokio::test]
async fn don_tron_hang_san_va_dat_truoc_tach_hai_lan_giao() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 10);
        let phone = preorder_item(&shop, 200_000);
        // Chưa đồng ý điều kiện đặt trước: 422.
        let (cart, quote) = cart_of(&shop, &[(shirt, 1), (phone, 1)]).await;
        let buyer = Call::as_user(&shop, "buyer1");
        let (status, error) = checkout(&buyer, Some("k1"), body(&cart, &quote, "cod")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["errors"][0]["field"], "accept_preorder_terms");

        let id = place(&shop, &[(shirt, 1), (phone, 1)], "cod").await;
        // Lần giao 1: chỉ hàng sẵn, thu hộ đúng tiền của nó.
        let (status, first) = ship(&shop, id).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{first}");
        assert_eq!(minor(&first["data"]["cod_amount"]), 100_000);
        let key = first["data"]["request_key"].as_str().unwrap().to_owned();
        // Gửi lại: cùng yêu cầu.
        let (_, again) = ship(&shop, id).await;
        assert_eq!(again["data"]["request_key"], key.as_str());
        // Kiện đang đi đường: chưa đơn vị nào tới tay người mua, đơn chưa giao một phần.
        let moving = keyed_shipment_event(id, &key, "in_transit", None);
        emit_from(&shop, &shop.shipping_outbox, &ShipmentStatusChanged(moving)).await;
        drain(&shop).await;
        assert_eq!(status_of(&shop, id).await, "shipping");
        let (_, found) = Call::as_user(&shop, "boss").send("GET", &format!("/{id}"), None).await;
        assert_eq!(found["data"]["fulfillment_status"], "unfulfilled", "{found}");
        deliver(&shop, id, &key, Some(100_000)).await;
        // Còn hàng đặt trước chưa giao: đơn chưa `delivered`.
        assert_eq!(status_of(&shop, id).await, "shipping");
        let (_, found) = Call::as_user(&shop, "boss").send("GET", &format!("/{id}"), None).await;
        assert_eq!(found["data"]["payment_status"], "partially_paid");
        // Lần giao lẻ tới tay người mua: báo kho xuất đúng phần đó (UT-1), đơn giao một phần.
        assert_eq!(count_events(&shop, "orders.fulfillment_delivered").await, 1);
        let shirt_line = line_of(&shop, id, shirt).await;
        let arrived = fulfillment_delivered(&shop).await;
        assert_eq!(arrived["lines"], json!([{"line_key": shirt_line.line_key, "quantity": 1, "serials": []}]), "{arrived}");
        assert_eq!(found["data"]["fulfillment_status"], "partial");
        // Hàng chưa về: không có gì để giao.
        let (status, error) = ship(&shop, id).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        assert_eq!(error["code"], "orders.nothing_to_ship");

        // Hàng về, được chia cho dòng: lần giao 2 thu phần còn lại (hàng + phí giao).
        let pre = line_of(&shop, id, phone).await;
        let allocated = json!({"after": {"order_id": id, "line_key": pre.line_key, "variant_id": phone,
            "warehouse_org_id": shop.tree.a, "quantity": 1, "allocated_quantity": 1}});
        emit_from(&shop, &shop.inventory_outbox, &PreorderAllocated(allocated)).await;
        drain(&shop).await;
        let (status, second) = ship(&shop, id).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{second}");
        assert_ne!(second["data"]["request_key"].as_str().unwrap(), key);
        assert_eq!(minor(&second["data"]["cod_amount"]), 230_000);
        let key2 = second["data"]["request_key"].as_str().unwrap().to_owned();
        deliver(&shop, id, &key2, Some(230_000)).await;
        assert_eq!(status_of(&shop, id).await, "delivered");
        let (_, found) = Call::as_user(&shop, "boss").send("GET", &format!("/{id}"), None).await;
        assert_eq!(found["data"]["payment_status"], "paid");
        // Lần cuối cũng có event riêng; `orders.order_delivered` không còn phần nào chưa đi theo lần giao.
        assert_eq!(count_events(&shop, "orders.fulfillment_delivered").await, 2);
        assert_eq!(count_events(&shop, "orders.order_delivered").await, 1);
        assert_eq!(found["data"]["fulfillment_status"], "fulfilled");
        assert_eq!(minor(&found["data"]["totals"]["paid"]), 330_000);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_ngay_dat_truoc_bao_nguoi_mua_va_cho_huy_mien_phi() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let phone = preorder_item(&shop, 200_000);
        let id = place(&shop, &[(phone, 1)], "cod").await;
        // Chính sách không cho người mua tự huỷ đơn đã xác nhận…
        shop.orgs.set(shop.tree.a, "orders.buyer_cancel_until", json!("pending_payment"));
        let buyer = Call::as_user(&shop, "buyer1");
        let (status, _) =
            buyer.send("POST", &format!("/me/orders/{id}/cancel"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        // …nhưng ngày giao dời muộn hơn ngày đã hứa: báo người mua, huỷ miễn phí.
        let later = (Timestamp::now() + SignedDuration::from_hours(24 * 40)).date_utc();
        let moved = json!({"before": {"id": phone, "sale_mode": "preorder"},
            "after": {"id": phone, "sale_mode": "preorder", "preorder_available_on": later}});
        emit_from(&shop, &shop.catalog_outbox, &PreorderDateChanged(moved.clone())).await;
        emit_from(&shop, &shop.catalog_outbox, &PreorderDateChanged(moved)).await;
        drain(&shop).await;
        assert_eq!(count_events(&shop, "orders.preorder_delayed").await, 1);
        let (status, cancelled) =
            buyer.send("POST", &format!("/me/orders/{id}/cancel"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
        assert_eq!(cancelled["data"]["status"], "cancelled");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ngung_dat_truoc_huy_va_hoan_tien_tu_dong() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 10);
        let phone = preorder_item(&shop, 200_000);
        // Đơn chỉ có hàng đặt trước, đã trả qua cổng: huỷ cả đơn, hoàn toàn bộ.
        let only = place(&shop, &[(phone, 1)], "card").await;
        // Đơn trộn, đã giao phần hàng sẵn: chỉ huỷ phần đặt trước, hoàn đúng tiền của nó.
        let mixed = place(&shop, &[(shirt, 1), (phone, 1)], "card").await;
        let (_, first) = ship(&shop, mixed).await;
        let key = first["data"]["request_key"].as_str().unwrap().to_owned();
        assert_eq!(first["data"]["cod_amount"], Value::Null);

        let stopped = json!({"before": {"id": phone, "sale_mode": "preorder", "active": true},
            "after": {"id": phone, "sale_mode": "preorder", "active": false}});
        emit_from(&shop, &shop.catalog_outbox, &VariantUpdated(stopped)).await;
        drain(&shop).await;
        assert_eq!(status_of(&shop, only).await, "cancelled");
        let refunds = super::refund_rows(&shop.ledger);
        let amounts: Vec<i64> = refunds.iter().map(|refund| refund.1).collect();
        assert!(amounts.contains(&200_000), "{refunds:?}");
        assert_eq!(count_events(&shop, "orders.preorder_cancelled").await, 1);
        assert_eq!(line_of(&shop, mixed, phone).await.cancelled_quantity, 1);
        let (_, found) = Call::as_user(&shop, "boss").send("GET", &format!("/{mixed}"), None).await;
        assert_eq!(found["data"]["payment_status"], "partially_refunded");
        // Phần còn lại đã giao đủ: đơn `delivered`.
        deliver(&shop, mixed, &key, None).await;
        assert_eq!(status_of(&shop, mixed).await, "delivered");
    })
    .await
    .unwrap();
}

/// `after` của event `orders.fulfillment_delivered` (một event).
async fn fulfillment_delivered(shop: &Shop) -> serde_json::Value {
    use sea_orm::ConnectionTrait as _;
    let read = shop.database().read();
    let sql = "SELECT payload FROM orders_events WHERE name = $1";
    let statement = autoxide_kernel::database::statement(
        read.get_database_backend(),
        sql,
        ["orders.fulfillment_delivered".into()],
    );
    let row = read.query_one_raw(statement).await.unwrap().unwrap();
    let payload: serde_json::Value =
        serde_json::from_str(&row.try_get_by_index::<String>(0).unwrap()).unwrap();
    payload["after"].clone()
}
