//! UT-6 (báo cáo dùng thử RC vòng 1): `ledger_payment_id` mở được ở `ledger` (L2), header
//! `Idempotent-Replayed` khi gửi lại cùng khoá (L3), mở lại sau lượt thu hỏng ra lần thử mới và lượt
//! cũ thành công muộn thì hoàn qua đối tác đúng một lần (L4).

use super::CatalogExt as _;
use autoxide_kernel::database::statement;
use autoxide_ledger::api::Ledger as _;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use sea_orm::ConnectionTrait as _;
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::{body, ready_cart};
use super::guests::{access_token, guests};
use super::{Call, Shop, drain};

/// Một request tới `/api/v1/orders…`: trả (status, header `Idempotent-Replayed`, thân).
async fn send(
    shop: &Shop,
    uri: &str,
    headers: &[(&str, &str)],
    body: Value,
) -> (StatusCode, Option<String>, Value) {
    let mut request = shop.request(axum::http::Method::POST, &format!("/api/v1/orders{uri}"));
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let answer = request.json(body).send().await.unwrap();
    let replayed =
        answer.headers().get("idempotent-replayed").map(|value| value.to_str().unwrap().to_owned());
    let json = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), replayed, json)
}

/// Đơn vãng lai trả qua cổng (`card`): (id đơn, số đơn, kết quả đặt hàng).
async fn card_order(shop: &Shop) -> (String, String, Value) {
    let shirt = shop.catalog.add(shop.tree.a, 200_000);
    shop.inventory.stock(shop.tree.a, shirt, 5);
    let guest = Call::as_guest(shop, None);
    let (cart, token, quote) = ready_cart(&guest, shop.tree.a, shirt, 1).await;
    let token = token.unwrap();
    let key = Uuid::now_v7().to_string();
    let (status, replayed, placed) = send(
        shop,
        "/checkout",
        &[("Cart-Token", &token), ("Idempotency-Key", &key)],
        body(&cart, &quote, "card"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    assert_eq!(replayed, None, "the first checkout is not a replay");
    // Gửi lại cùng khoá: 200, `Idempotent-Replayed: true`, đúng thân của lần đầu.
    let (status, replayed, again) = send(
        shop,
        "/checkout",
        &[("Cart-Token", &token), ("Idempotency-Key", &key)],
        body(&cart, &quote, "card"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(replayed.as_deref(), Some("true"));
    assert_eq!(again, placed);
    let order = &placed["data"]["orders"][0];
    let id = order["id"].as_str().unwrap().to_owned();
    let number = order["number"].as_str().unwrap().to_owned();
    (id, number, placed)
}

/// Số event `orders.payment_recorded` mang `superseded: true` của khách mặc định.
async fn superseded_events(shop: &Shop) -> usize {
    let database = shop.database();
    let backend = database.read().get_database_backend();
    let sql = "SELECT payload FROM orders_events WHERE name = 'orders.payment_recorded'";
    let rows = database.read().query_all_raw(statement(backend, sql, [])).await.unwrap();
    rows.iter()
        .map(|row| row.try_get::<String>("", "payload").unwrap())
        .filter(|payload| {
            serde_json::from_str::<Value>(payload).unwrap()["after"]["superseded"] == true
        })
        .count()
}

#[tokio::test]
async fn ma_luot_thu_cua_ledger_mo_duoc_va_goi_lai_co_header() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let shop = &guests.shop;
        let (order, number, placed) = card_order(shop).await;

        // L2: `payment_id` là khoản của đơn; `ledger_payment_id` là lượt thu ở `ledger`.
        let action = &placed["data"]["payment_actions"][0];
        let ledger_id: Uuid = action["ledger_payment_id"].as_str().unwrap().parse().unwrap();
        assert_ne!(action["payment_id"], action["ledger_payment_id"]);
        let found = shop.ledger.payment(&shop.default_tenant(), ledger_id).await.unwrap();
        assert_eq!(found.map(|payment| payment.reference), Some(format!("orders:{order}:order:1")));

        // L3: huỷ đơn vãng lai gửi lại cùng khoá → header và cùng thân.
        let token = access_token(&guests, &number).await;
        let uri = format!("/public/orders/{order}/cancel");
        let headers = [("Order-Token", token.as_str()), ("Idempotency-Key", "cancel-1")];
        let (status, replayed, cancelled) =
            send(shop, &uri, &headers, json!({"reason": "đổi ý"})).await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
        assert_eq!(replayed, None);
        let (status, replayed, again) =
            send(shop, &uri, &headers, json!({"reason": "đổi ý"})).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(replayed.as_deref(), Some("true"));
        assert_eq!(again, cancelled);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn luot_thu_hong_thi_mo_lai_ra_lan_thu_moi_va_luot_cu_thu_muon_duoc_hoan() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let shop = &guests.shop;
        let tenant = shop.default_tenant();
        let (order, number, placed) = card_order(shop).await;
        let first = placed["data"]["payment_actions"][0].clone();
        let old = shop.ledger.payments()[0].clone();
        let token = access_token(&guests, &number).await;
        let uri = format!("/public/orders/{order}/payment-link");

        // Lượt thu còn chờ: mở lại trả đúng lượt đó.
        let (status, _, same) =
            send(shop, &uri, &[("Order-Token", &token), ("Idempotency-Key", "l1")], json!({}))
                .await;
        assert_eq!(status, StatusCode::OK, "{same}");
        assert_eq!(same["data"][0]["payment_id"], first["payment_id"]);
        assert_eq!(shop.ledger.payments().len(), 1);

        // Đối tác báo hỏng: mở lại ra khoản mới, lượt thu mới, mã tham chiếu lần 2.
        shop.ledger.fail(old.id);
        let headers = [("Order-Token", token.as_str()), ("Idempotency-Key", "l2")];
        let (status, replayed, opened) = send(shop, &uri, &headers, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{opened}");
        assert_eq!(replayed, None);
        let action = &opened["data"][0];
        assert_eq!(opened["data"].as_array().unwrap().len(), 1, "{opened}");
        assert_ne!(action["payment_id"], first["payment_id"]);
        assert_ne!(action["status_token"], first["status_token"]);
        assert_eq!(action["kind"], "redirect", "{action}");
        let new = shop.ledger.last_payment().unwrap();
        assert_ne!(new.id, old.id);
        assert_eq!(new.reference, format!("orders:{order}:order:2"));
        assert_eq!(action["ledger_payment_id"], json!(new.id));
        // Gửi lại cùng khoá: cùng khoản mới, có header, không tạo lượt thu nào nữa.
        let (status, replayed, again) = send(shop, &uri, &headers, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(replayed.as_deref(), Some("true"));
        assert_eq!(again, opened);
        assert_eq!(shop.ledger.payments().len(), 2);

        // Thu lượt mới: đơn xác nhận.
        shop.ledger.succeed(&tenant, &new).await.unwrap();
        drain(shop).await;
        let boss = Call::as_user(shop, "boss");
        let (_, view) = boss.send("GET", &format!("/{order}"), None).await;
        assert_eq!(view["data"]["status"], "confirmed", "{view}");
        assert_eq!(super::minor(&view["data"]["totals"]["paid"]), new.amount.minor());

        // Đối tác vẫn báo đã thu lượt cũ (hai lần): hoàn qua đối tác đúng một lần, đơn không cộng thêm,
        // một event audit.
        shop.ledger.succeed(&tenant, &old).await.unwrap();
        drain(shop).await;
        shop.ledger.succeed(&tenant, &old).await.unwrap();
        drain(shop).await;
        assert_eq!(
            super::refund_rows(&shop.ledger),
            vec![(old.id, old.amount.minor(), "superseded_payment".to_owned())]
        );
        let (_, view) = boss.send("GET", &format!("/{order}"), None).await;
        assert_eq!(super::minor(&view["data"]["totals"]["paid"]), new.amount.minor());
        assert_eq!(superseded_events(shop).await, 1);
    })
    .await
    .unwrap();
}
