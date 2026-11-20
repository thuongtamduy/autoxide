//! Đổi trả và hoàn tiền: người mua yêu cầu trả trong hạn với lý do cho phép, nhân viên duyệt / nhận
//! hàng / hoàn, tiền hoàn theo tỉ lệ, nhập lại kho qua event, bút toán đảo cân, yêu cầu mở hoãn hoàn
//! tất; nhiều lệnh hoàn song song không vượt tiền đã thu.

use autoxide_kernel::tenancy::ScopeExt;
use autoxide_ledger::api::Side;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use futures_util::future::join_all;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::{body, checkout, ready_cart};
use super::lifecycle::{cod_order, count_events, status_of};
use super::{
    Call, CodCollected, PaymentSucceeded, ShipmentDelivered, Shop, drain, emit, emit_from, minor,
    run_job, shipment_event, shop,
};
use crate::entity::{line, order};

/// Đơn COD (2 × 100.000 + phí giao 30.000) đã giao và đã thu tiền; trả (ID đơn, ID dòng áo).
pub(super) async fn delivered_order(shop: &Shop) -> (Uuid, Uuid) {
    let id = cod_order(shop).await;
    deliver(shop, id).await;
    let (_, found) = Call::as_user(shop, "boss").send("GET", &format!("/{id}"), None).await;
    let line_id = found["data"]["lines"][0]["id"].as_str().unwrap().parse().unwrap();
    (id, line_id)
}

/// Hãng báo đã giao đơn COD `id` và đã thu tiền.
pub(super) async fn deliver(shop: &Shop, id: Uuid) {
    emit_from(
        shop,
        &shop.shipping_outbox,
        &ShipmentDelivered(shipment_event(id, "delivered", None)),
    )
    .await;
    emit_from(
        shop,
        &shop.shipping_outbox,
        &CodCollected(shipment_event(id, "delivered", Some(230_000))),
    )
    .await;
    drain(shop).await;
    assert_eq!(status_of(shop, id).await, "delivered");
}

pub(super) fn return_body(line: Uuid, quantity: i64, reason: &str) -> Value {
    json!({"lines": [{"line_id": line, "quantity": quantity}], "reason_code": reason})
}

pub(super) async fn step(shop: &Shop, id: &str, body: Value) -> (StatusCode, Value) {
    Call::as_user(shop, "boss")
        .send("POST", &format!("/returns/{id}/transitions"), Some(body))
        .await
}

#[tokio::test]
async fn partial_return_refunds_proportionally_restocks_and_reverses_revenue() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (id, shirt) = delivered_order(&shop).await;
        let buyer = Call::as_user(&shop, "buyer1");
        let uri = format!("/me/orders/{id}/returns");

        // Lý do cửa hàng không nhận, số lượng vượt: một lỗi 422 gom cả hai.
        shop.orgs.set(shop.tree.a, "orders.allowed_return_reasons", json!(["damaged"]));
        let (status, error) =
            buyer.send("POST", &uri, Some(return_body(shirt, 3, "changed_mind"))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        let fields: Vec<&str> = error["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert!(
            fields.contains(&"reason_code") && fields.contains(&"lines[0].quantity"),
            "{error}"
        );
        shop.orgs.set(
            shop.tree.a,
            "orders.allowed_return_reasons",
            json!(["changed_mind", "damaged"]),
        );
        // Người khác không trả được đơn của mình.
        let (status, _) = Call::as_user(&shop, "buyer2")
            .send("POST", &uri, Some(return_body(shirt, 1, "changed_mind")))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, asked) =
            buyer.send("POST", &uri, Some(return_body(shirt, 1, "changed_mind"))).await;
        assert_eq!(status, StatusCode::CREATED, "{asked}");
        assert_eq!(asked["data"]["status"], "requested");
        assert_eq!(asked["data"]["return_method"], "drop_off");
        let return_id = asked["data"]["id"].as_str().unwrap().to_owned();
        // Đơn vị còn lại 1 (1 đang trong yêu cầu mở): xin 2 nữa thì 422.
        let (status, _) =
            buyer.send("POST", &uri, Some(return_body(shirt, 2, "changed_mind"))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(count_events(&shop, "orders.return_requested").await, 1);

        // Yêu cầu mở hoãn hoàn tất đơn dù đã quá hạn đổi trả.
        let tenant = shop.default_tenant();
        order::Entity::update_many()
            .col_expr(order::Column::DeliveredAt, Expr::value(Utc::now() - Duration::days(8)))
            .filter(order::Column::Id.eq(id))
            .scoped(&tenant)
            .exec(shop.database().write())
            .await
            .unwrap();
        run_job(&shop, crate::jobs::COMPLETE_DELIVERED).await;
        assert_eq!(status_of(&shop, id).await, "delivered");
        // Đã quá hạn: yêu cầu mới bị chặn.
        let (status, error) =
            buyer.send("POST", &uri, Some(return_body(shirt, 1, "damaged"))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        assert_eq!(error["code"], "orders.return_window_closed");

        // Chưa nhận hàng thì chưa hoàn được; nhân viên không có quyền thì 403.
        let (status, error) = step(&shop, &return_id, json!({"action": "refund"})).await;
        assert_eq!(status, StatusCode::CONFLICT, "{error}");
        assert_eq!(error["code"], "orders.return_invalid_transition");
        let (status, _) = Call::as_user(&shop, "staff_a")
            .send(
                "POST",
                &format!("/returns/{return_id}/transitions"),
                Some(json!({"action": "approve"})),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, approved) = step(&shop, &return_id, json!({"action": "approve"})).await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        let (status, received) = step(&shop, &return_id, json!({"action": "receive"})).await;
        assert_eq!(status, StatusCode::OK, "{received}");
        assert_eq!(received["data"]["status"], "received");
        // Đổi ý: hoàn tiền hàng của 1 đơn vị, không hoàn phí giao.
        assert_eq!(minor(&received["data"]["refund"]), 100_000);
        assert_eq!(minor(&received["data"]["refund_breakdown"]["shipping"]), 0);
        assert_eq!(count_events(&shop, "orders.return_received").await, 1);
        let returned =
            line::Entity::find_by_id(shirt).one(shop.database().read()).await.unwrap().unwrap();
        assert_eq!(returned.returned_quantity, 1);

        let (status, refunded) = step(&shop, &return_id, json!({"action": "refund"})).await;
        assert_eq!(status, StatusCode::OK, "{refunded}");
        assert_eq!(refunded["data"]["status"], "refunded");
        // Gọi lại: không hoàn hai lần.
        let (status, _) = step(&shop, &return_id, json!({"action": "refund"})).await;
        assert_eq!(status, StatusCode::OK);
        drain(&shop).await;
        let (_, refunds) =
            Call::as_user(&shop, "boss").send("GET", &format!("/{id}/refunds"), None).await;
        let refunds = refunds["data"].as_array().unwrap().clone();
        assert_eq!(refunds.len(), 1);
        // COD: người bán tự trả tiền, lượt hoàn ghi `manual`.
        assert_eq!(refunds[0]["status"], "manual");
        let (_, found) = Call::as_user(&shop, "boss").send("GET", &format!("/{id}"), None).await;
        assert_eq!(found["data"]["payment_status"], "partially_refunded");
        assert_eq!(minor(&found["data"]["totals"]["refunded"]), 100_000);
        assert_eq!(count_events(&shop, "orders.refund_completed").await, 1);

        // Bút toán của lượt hoàn: trả tiền và đảo doanh thu, mỗi bút toán Nợ = Có = 100.000.
        let refund_id = refunds[0]["id"].as_str().unwrap();
        let entries = shop.ledger.entries.lock().unwrap().clone();
        for suffix in ["paid", "revenue"] {
            let reference = format!("orders:{id}:refund:{refund_id}:{suffix}");
            let posted: Vec<_> = entries.iter().filter(|(r, _)| *r == reference).collect();
            assert_eq!(posted.len(), 1, "{reference}");
            let side = |wanted: Side| -> i64 {
                posted[0].1.iter().filter(|l| l.1 == wanted).map(|l| l.2).sum()
            };
            assert_eq!((side(Side::Debit), side(Side::Credit)), (100_000, 100_000), "{suffix}");
        }
        // Đã hoàn xong: không còn hoãn, đơn hoàn tất.
        run_job(&shop, crate::jobs::COMPLETE_DELIVERED).await;
        assert_eq!(status_of(&shop, id).await, "completed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn seller_fault_return_refunds_shipping_and_reject_needs_listed_reason() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (id, shirt) = delivered_order(&shop).await;
        let buyer = Call::as_user(&shop, "buyer1");
        let uri = format!("/me/orders/{id}/returns");
        let (status, asked) = buyer.send("POST", &uri, Some(return_body(shirt, 1, "changed_mind"))).await;
        assert_eq!(status, StatusCode::CREATED, "{asked}");
        let first = asked["data"]["id"].as_str().unwrap().to_owned();
        let (status, error) = step(&shop, &first, json!({"action": "reject", "reason": "not_a_reason"})).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        let (status, rejected) = step(&shop, &first, json!({"action": "reject", "reason": "outside_policy"})).await;
        assert_eq!(status, StatusCode::OK, "{rejected}");
        assert_eq!(rejected["data"]["reject_reason"], "outside_policy");

        // Hàng hỏng (lỗi người bán), trả cả 2: hoàn đủ tiền hàng và phí giao = đã trả 230.000.
        let mut damaged = return_body(shirt, 2, "damaged");
        damaged["media_ids"] = json!([shop.media.upload(Uuid::from_u128(101))]);
        let (status, asked) = buyer.send("POST", &uri, Some(damaged)).await;
        assert_eq!(status, StatusCode::CREATED, "{asked}");
        let second = asked["data"]["id"].as_str().unwrap().to_owned();
        let (status, received) = step(
            &shop,
            &second,
            json!({"action": "receive", "lines": [{"line_id": shirt, "quantity": 2, "condition": "damaged"}]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{received}");
        assert_eq!(minor(&received["data"]["refund"]), 230_000);
        assert_eq!(minor(&received["data"]["refund_breakdown"]["shipping"]), 30_000);
        assert_eq!(received["data"]["lines"][0]["condition"], "damaged");
        let (status, _) = step(&shop, &second, json!({"action": "refund"})).await;
        assert_eq!(status, StatusCode::OK);
        let (status, closed) = step(&shop, &second, json!({"action": "close"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(closed["data"]["status"], "closed");
        let (_, found) = Call::as_user(&shop, "boss").send("GET", &format!("/{id}"), None).await;
        assert_eq!(found["data"]["payment_status"], "refunded");
        // Hết hàng để trả.
        let (status, _) = buyer.send("POST", &uri, Some(return_body(shirt, 1, "damaged"))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (_, list) = Call::as_user(&shop, "boss").send("GET", &format!("/returns?order_id={id}"), None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn concurrent_refunds_never_exceed_what_was_paid() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 5);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 2).await;
        let (status, placed) = checkout(&buyer, Some("pay"), body(&cart, &quote, "card")).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        let id: Uuid = placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap();
        let payment = shop.ledger.payments.lock().unwrap().last().cloned().unwrap();
        emit(&shop, &PaymentSucceeded(json!({"payment_id": payment.id, "source_module": "orders", "amount": payment.amount}))).await;
        drain(&shop).await;
        let paid = payment.amount.minor();
        let quarter = paid / 4;

        let boss = Call::as_user(&shop, "boss");
        let calls = (0..10).map(|n| {
            let boss = &boss;
            async move {
                boss.send(
                    "POST",
                    &format!("/{id}/refunds"),
                    Some(json!({"amount": {"amount": quarter.to_string(), "currency": "VND"}, "reason": "goodwill", "reference": format!("r{n}")})),
                )
                .await
            }
        });
        let answers = join_all(calls).await;
        let created = answers.iter().filter(|(status, _)| *status == StatusCode::CREATED).count();
        assert_eq!(created, 4, "{answers:?}");
        for (status, answer) in &answers {
            if *status != StatusCode::CREATED {
                assert_eq!(*status, StatusCode::CONFLICT, "{answer}");
                assert_eq!(answer["code"], "orders.refund_exceeds_paid");
            }
        }
        // Gửi lại cùng mã của một lượt đã tạo: lượt cũ (200), không hoàn thêm.
        let winner = answers.iter().position(|(status, _)| *status == StatusCode::CREATED).unwrap();
        let (status, _) = boss
            .send(
                "POST",
                &format!("/{id}/refunds"),
                Some(json!({"amount": {"amount": quarter.to_string(), "currency": "VND"}, "reason": "goodwill", "reference": format!("r{winner}")})),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let refunded: i64 = shop.ledger.refunds.lock().unwrap().iter().map(|r| r.1).sum();
        assert!(refunded <= paid);
        assert_eq!(refunded, quarter * 4);
        // Không có `orders.refund`: 403.
        let (status, _) = Call::as_user(&shop, "staff_a")
            .send(
                "POST",
                &format!("/{id}/refunds"),
                Some(json!({"amount": {"amount": "1", "currency": "VND"}, "reason": "x", "reference": "s1"})),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn preview_shows_refund_methods_and_eta_without_saving_and_photos_are_checked() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let (id, shirt) = delivered_order(&shop).await;
        let buyer = Call::as_user(&shop, "buyer1");
        let preview = format!("/me/orders/{id}/returns/preview");
        let (status, seen) =
            buyer.send("POST", &preview, Some(return_body(shirt, 1, "changed_mind"))).await;
        assert_eq!(status, StatusCode::OK, "{seen}");
        assert_eq!(minor(&seen["data"]["refund"]), 100_000);
        assert_eq!(seen["data"]["photos_required"], false);
        // COD: tiền hoàn chuyển khoản về người mua.
        assert_eq!(seen["data"]["refund_eta"]["channel"], "bank_transfer");
        let methods = seen["data"]["methods"].as_array().unwrap();
        let method = |name: &str| methods.iter().find(|m| m["method"] == name).unwrap().clone();
        assert_eq!(method("drop_off")["available"], true);
        // Không phải lỗi người bán: người mua trả phí hãng lấy hàng.
        assert_eq!(method("pickup")["available"], true);
        assert_eq!(minor(&method("pickup")["fee"]), 30_000);
        // Đơn không có cửa hàng nhận: không trả tại cửa hàng được.
        assert_eq!(method("in_store")["available"], false);
        // Không ghi gì.
        let (_, list) = Call::as_user(&shop, "boss")
            .send("GET", &format!("/returns?order_id={id}"), None)
            .await;
        assert!(list["data"].as_array().unwrap().is_empty());
        assert_eq!(count_events(&shop, "orders.return_requested").await, 0);

        // Hàng hỏng cần ảnh; ảnh của người khác bị từ chối — cùng lỗi ở xem trước và khi gửi.
        let (status, error) =
            buyer.send("POST", &preview, Some(return_body(shirt, 1, "damaged"))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["errors"][0]["field"], "media_ids");
        let mine = shop.media.upload(Uuid::from_u128(101));
        let theirs = shop.media.upload(Uuid::from_u128(102));
        let mut body = return_body(shirt, 1, "damaged");
        body["media_ids"] = json!([mine, theirs]);
        let (status, error) =
            buyer.send("POST", &format!("/me/orders/{id}/returns"), Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(error["errors"][0]["field"], "media_ids[1]");
        // Ảnh của mình, hãng tới lấy: lỗi người bán nên phí lấy hàng 0.
        body["media_ids"] = json!([mine]);
        body["return_method"] = json!("pickup");
        let (status, seen) = buyer.send("POST", &preview, Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{seen}");
        assert_eq!(seen["data"]["photos_required"], true);
        let (status, asked) =
            buyer.send("POST", &format!("/me/orders/{id}/returns"), Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{asked}");
        assert_eq!(asked["data"]["media_ids"], json!([mine]));
        let return_id = asked["data"]["id"].as_str().unwrap().to_owned();
        let (status, _) = step(&shop, &return_id, json!({"action": "approve"})).await;
        assert_eq!(status, StatusCode::OK);
        // Duyệt trả hàng lấy tận nơi: yêu cầu hãng lấy hàng chiều về, đọc được bằng `ShipmentRequest`.
        let read = shop.database().read();
        let row = read
            .query_one_raw(sea_orm::Statement::from_string(
                read.get_database_backend(),
                "SELECT payload FROM orders_events WHERE name = 'orders.return_pickup_requested'",
            ))
            .await
            .unwrap()
            .unwrap();
        let payload: Value =
            serde_json::from_str(&row.try_get_by_index::<String>(0).unwrap()).unwrap();
        let shipment: autoxide_shipping::api::ShipmentRequest =
            serde_json::from_value(payload["after"]["shipment"].clone()).unwrap();
        assert_eq!(shipment.direction, autoxide_shipping::api::Direction::Return);
        assert_eq!(shipment.items[0].quantity, 1);
        assert_eq!(shipment.request_key, format!("return:{return_id}"));
    })
    .await
    .unwrap();
}
