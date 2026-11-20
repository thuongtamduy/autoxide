//! Vòng đời đơn: nhân viên chuyển trạng thái (`If-Match`), người mua huỷ theo chính sách, hoàn tiền
//! khi huỷ đơn đã thu, yêu cầu vận đơn, event của `shipping`, bút toán doanh thu cân, hoàn tất khi
//! hết hạn đổi trả (trừ khi bị hoãn), huỷ đơn cửa hàng không nhận kịp.

use autoxide_kernel::tenancy::ScopeExt;
use autoxide_ledger::api::Side;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::{body, checkout, ready_cart};
use super::{
    Call, CodCollected, PaymentSucceeded, ShipmentDelivered, Shop, drain, emit, emit_from,
    keyed_shipment_event, run_job, shop,
};
use crate::entity::order;

/// Đơn COD của `buyer1` ở cửa hàng A (2 món × 100.000 + phí giao 30.000); trả ID.
pub(super) async fn cod_order(shop: &Shop) -> Uuid {
    let shirt = shop.catalog.add(shop.tree.a, 100_000);
    shop.inventory.stock(shop.tree.a, shirt, 10);
    let buyer = Call::as_user(shop, "buyer1");
    let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 2).await;
    let (status, placed) =
        checkout(&buyer, Some(&Uuid::now_v7().to_string()), body(&cart, &quote, "cod")).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap()
}

/// `POST /api/v1/orders/{id}/transitions` của `boss` với `If-Match`.
pub(super) async fn transition(
    shop: &Shop,
    id: Uuid,
    to: &str,
    if_match: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = shop
        .post(&format!("/api/v1/orders/{id}/transitions"))
        .token("boss")
        .json(json!({"to": to}));
    if let Some(etag) = if_match {
        request = request.header("If-Match", etag);
    }
    let answer = request.send().await.unwrap();
    (answer.status(), answer.json())
}

pub(super) async fn etag(shop: &Shop, id: Uuid) -> String {
    let answer = shop.get(&format!("/api/v1/orders/{id}")).token("boss").send().await.unwrap();
    assert_eq!(answer.status(), StatusCode::OK);
    answer.header("etag").to_owned()
}

pub(super) async fn status_of(shop: &Shop, id: Uuid) -> String {
    let (_, found) = Call::as_user(shop, "boss").send("GET", &format!("/{id}"), None).await;
    found["data"]["status"].as_str().unwrap().to_owned()
}

pub(super) async fn count_events(shop: &Shop, name: &str) -> i64 {
    let read = shop.database().read();
    let sql = format!("SELECT COUNT(*) FROM orders_events WHERE name = '{name}'");
    let row = read
        .query_one_raw(Statement::from_string(read.get_database_backend(), sql))
        .await
        .unwrap()
        .unwrap();
    row.try_get_by_index::<i64>(0).unwrap()
}

#[tokio::test]
async fn staff_moves_order_with_if_match_and_table_rejects_others() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let id = cod_order(&shop).await;
        let (status, error) = transition(&shop, id, "preparing", None).await;
        assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{error}");
        let stale = etag(&shop, id).await;
        let (status, moved) = transition(&shop, id, "preparing", Some(&stale)).await;
        assert_eq!(status, StatusCode::OK, "{moved}");
        assert_eq!(moved["data"]["status"], "preparing");
        // Bản đã đọc cũ rồi: 412.
        let (status, _) = transition(&shop, id, "shipping", Some(&stale)).await;
        assert_eq!(status, StatusCode::PRECONDITION_FAILED);
        // Không có trong bảng chuyển: 409.
        let (status, error) = transition(&shop, id, "completed", Some("*")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["code"], "orders.invalid_transition");
        let (status, error) = transition(&shop, id, "pending_payment", Some("*")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        // Không có `orders.manage`: 403.
        let answer = shop
            .post(&format!("/api/v1/orders/{id}/transitions"))
            .token("staff_a")
            .header("If-Match", "*")
            .json(json!({"to": "shipping"}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn buyer_cancels_per_policy_and_paid_order_is_refunded() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let buyer = Call::as_user(&shop, "buyer1");
        // COD đã xác nhận: chính sách mặc định cho huỷ tới `confirmed`.
        let id = cod_order(&shop).await;
        let (status, cancelled) = buyer.send("POST", &format!("/me/orders/{id}/cancel"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
        assert_eq!(cancelled["data"]["status"], "cancelled");
        drain(&shop).await;
        // Cửa hàng đã nhận đơn (`preparing`): người mua không tự huỷ được nữa…
        let second = cod_order(&shop).await;
        transition(&shop, second, "preparing", Some("*")).await;
        let (status, error) = buyer.send("POST", &format!("/me/orders/{second}/cancel"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(error["code"], "orders.cancel_not_allowed");
        // …trừ khi chính sách của cửa hàng cho tới `preparing`.
        shop.orgs.set(shop.tree.a, "orders.buyer_cancel_until", json!("preparing"));
        let (status, _) = buyer.send("POST", &format!("/me/orders/{second}/cancel"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK);
        // Người khác không huỷ được đơn của mình.
        let third = cod_order(&shop).await;
        let (status, _) = Call::as_user(&shop, "buyer2").send("POST", &format!("/me/orders/{third}/cancel"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Đơn đã trả qua cổng, nhân viên huỷ: hoàn toàn bộ qua `ledger`.
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 5);
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        let (_, placed) = checkout(&buyer, Some("pay"), body(&cart, &quote, "card")).await;
        let paid: Uuid = placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap();
        let payment = shop.ledger.payments.lock().unwrap().last().cloned().unwrap();
        emit(&shop, &PaymentSucceeded(json!({"payment_id": payment.id, "source_module": "orders", "amount": payment.amount}))).await;
        drain(&shop).await;
        let (status, moved) = transition(&shop, paid, "cancelled", Some("*")).await;
        assert_eq!(status, StatusCode::OK, "{moved}");
        assert_eq!(moved["data"]["payment_status"], "refunded");
        assert_eq!(super::minor(&moved["data"]["totals"]["refunded"]), payment.amount.minor());
        let refunds = shop.ledger.refunds.lock().unwrap().clone();
        assert_eq!(refunds, vec![(payment.id, payment.amount.minor(), "order_cancelled".to_owned())]);
        assert_eq!(count_events(&shop, "orders.refund_completed").await, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn shipment_request_and_carrier_events_drive_the_order_and_post_balanced_revenue() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let id = cod_order(&shop).await;
        let (status, requested) = Call::as_user(&shop, "boss")
            .send("POST", &format!("/{id}/shipments"), Some(json!({})))
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{requested}");
        assert_eq!(super::minor(&requested["data"]["cod_amount"]), 230_000);
        assert_eq!(status_of(&shop, id).await, "preparing");
        assert_eq!(count_events(&shop, "orders.fulfillment_requested").await, 1);
        // `after.shipment` đọc được thành `ShipmentRequest` của `shipping`.
        let read = shop.database().read();
        let row = read
            .query_one_raw(Statement::from_string(
                read.get_database_backend(),
                "SELECT payload FROM orders_events WHERE name = 'orders.fulfillment_requested'",
            ))
            .await
            .unwrap()
            .unwrap();
        let payload: Value =
            serde_json::from_str(&row.try_get_by_index::<String>(0).unwrap()).unwrap();
        let shipment: autoxide_shipping::api::ShipmentRequest =
            serde_json::from_value(payload["after"]["shipment"].clone()).unwrap();
        assert_eq!(shipment.order_id, id);
        assert_eq!(shipment.items[0].quantity, 2);
        assert_eq!(shipment.recipient_phone, "0901234567");

        // Hãng báo đã giao tới trước "đã lấy hàng", giao hai lần: đơn `delivered` một lần.
        let key = requested["data"]["request_key"].as_str().unwrap().to_owned();
        let delivered = keyed_shipment_event(id, &key, "delivered", None);
        emit_from(&shop, &shop.shipping_outbox, &ShipmentDelivered(delivered.clone())).await;
        emit_from(&shop, &shop.shipping_outbox, &ShipmentDelivered(delivered)).await;
        emit_from(
            &shop,
            &shop.shipping_outbox,
            &CodCollected(keyed_shipment_event(id, &key, "delivered", Some(230_000))),
        )
        .await;
        drain(&shop).await;
        assert_eq!(status_of(&shop, id).await, "delivered");
        assert_eq!(count_events(&shop, "orders.order_delivered").await, 1);
        let (_, found) = Call::as_user(&shop, "boss").send("GET", &format!("/{id}"), None).await;
        assert_eq!(found["data"]["payment_status"], "paid");
        assert_eq!(super::minor(&found["data"]["totals"]["paid"]), 230_000);
        // Bút toán doanh thu: một lần, Nợ = Có.
        let entries = shop.ledger.entries.lock().unwrap().clone();
        let revenue: Vec<_> = entries
            .iter()
            .filter(|(reference, _)| *reference == format!("orders:{id}:revenue"))
            .collect();
        assert_eq!(revenue.len(), 1);
        let debit: i64 =
            revenue[0].1.iter().filter(|line| line.1 == Side::Debit).map(|line| line.2).sum();
        let credit: i64 =
            revenue[0].1.iter().filter(|line| line.1 == Side::Credit).map(|line| line.2).sum();
        assert_eq!((debit, credit), (230_000, 230_000));

        // Hết hạn đổi trả: hoàn tất — trừ khi đang bị hoãn (tranh chấp, trả hàng).
        use crate::api::OrderHolds;
        let holds = shop.api::<dyn OrderHolds>().unwrap();
        let tenant = shop.default_tenant();
        holds.hold_completion(&tenant, id, "dispute:1").await.unwrap();
        holds.hold_completion(&tenant, id, "dispute:1").await.unwrap();
        order::Entity::update_many()
            .col_expr(order::Column::DeliveredAt, Expr::value(Utc::now() - Duration::days(8)))
            .filter(order::Column::Id.eq(id))
            .scoped(&tenant)
            .exec(shop.database().write())
            .await
            .unwrap();
        run_job(&shop, crate::jobs::COMPLETE_DELIVERED).await;
        assert_eq!(status_of(&shop, id).await, "delivered");
        holds.release_completion(&tenant, id, "dispute:1").await.unwrap();
        run_job(&shop, crate::jobs::COMPLETE_DELIVERED).await;
        assert_eq!(status_of(&shop, id).await, "completed");
        assert_eq!(count_events(&shop, "orders.order_completed").await, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn unaccepted_order_is_cancelled_after_acceptance_hours() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let id = cod_order(&shop).await;
        // Mặc định 0 giờ: không có bước nhận đơn, job không đụng tới.
        order::Entity::update_many()
            .col_expr(order::Column::ConfirmedAt, Expr::value(Utc::now() - Duration::hours(5)))
            .filter(order::Column::Id.eq(id))
            .exec(shop.database().write())
            .await
            .unwrap();
        run_job(&shop, crate::jobs::EXPIRE_UNACCEPTED).await;
        assert_eq!(status_of(&shop, id).await, "confirmed");
        shop.orgs.set(shop.tree.a, "orders.acceptance_hours", json!(4));
        run_job(&shop, crate::jobs::EXPIRE_UNACCEPTED).await;
        let (_, found) = Call::as_user(&shop, "boss").send("GET", &format!("/{id}"), None).await;
        assert_eq!(found["data"]["status"], "cancelled");
        assert_eq!(found["data"]["cancel_reason"], "seller_not_accepted");
        // COD không cho ở cửa hàng tắt COD.
        shop.orgs.set(shop.tree.a, "orders.cod_enabled", json!(false));
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 5);
        let buyer = Call::as_user(&shop, "buyer2");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        let (status, error) = checkout(&buyer, Some("cod"), body(&cart, &quote, "cod")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error["code"], "orders.cod_not_allowed");
    })
    .await
    .unwrap();
}
