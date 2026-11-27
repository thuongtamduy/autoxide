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
use super::{Call, PaymentSucceeded, Shop, drain, emit};

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

/// `ledger` báo đã thu đủ lượt `payment` (như `ledger.payment_succeeded` thật).
async fn paid(shop: &Shop, payment: &autoxide_ledger::api::Payment) {
    // Như `ledger` thật: ghi thu trước, phát event sau.
    shop.ledger.collect(payment.id);
    let event = json!({"payment_id": payment.id, "source_module": "orders",
        "amount": payment.amount, "reference": payment.reference});
    emit(shop, &PaymentSucceeded(event)).await;
}

/// Số event `ledger.*` chưa giao xong (bên nhận trả lỗi thì event nằm chờ thử lại).
async fn undelivered(shop: &Shop) -> i64 {
    let database = shop.database();
    let backend = database.read().get_database_backend();
    let sql = "SELECT COUNT(*) AS n FROM ledger_events WHERE delivered_at IS NULL";
    let row = database.read().query_one_raw(statement(backend, sql, [])).await.unwrap().unwrap();
    row.try_get::<i64>("", "n").unwrap()
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
        paid(shop, &new).await;
        drain(shop).await;
        let boss = Call::as_user(shop, "boss");
        let (_, view) = boss.send("GET", &format!("/{order}"), None).await;
        assert_eq!(view["data"]["status"], "confirmed", "{view}");
        assert_eq!(super::minor(&view["data"]["totals"]["paid"]), new.amount.minor());

        // Đối tác vẫn báo đã thu lượt cũ (hai lần): hoàn qua đối tác đúng một lần, đơn không cộng thêm,
        // một event audit.
        paid(shop, &old).await;
        drain(shop).await;
        paid(shop, &old).await;
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

/// Hạn thanh toán của đơn `order` (đọc qua API của nhân viên).
async fn due_of(shop: &Shop, order: &str) -> Value {
    let (_, view) = Call::as_user(shop, "boss").send("GET", &format!("/{order}"), None).await;
    view["data"]["payment_due_at"].clone()
}

#[tokio::test]
async fn mo_lai_nhieu_lan_khong_keo_dai_han_cua_don() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let shop = &guests.shop;
        let (order, number, _) = card_order(shop).await;
        let token = access_token(&guests, &number).await;
        let uri = format!("/public/orders/{order}/payment-link");
        let due = due_of(shop, &order).await;
        assert!(due.is_string(), "{due}");
        for attempt in 0..3 {
            let last = shop.ledger.last_payment().unwrap();
            shop.ledger.fail(last.id);
            // Lượt thu mới của bộ giả hết hạn muộn hơn lượt trước.
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            let key = format!("again-{attempt}");
            let (status, _, opened) =
                send(shop, &uri, &[("Order-Token", &token), ("Idempotency-Key", &key)], json!({}))
                    .await;
            assert_eq!(status, StatusCode::OK, "{opened}");
            assert_ne!(shop.ledger.last_payment().unwrap().id, last.id);
            assert_eq!(due_of(shop, &order).await, due, "lần thử {attempt} kéo dài hạn");
            assert_eq!(opened["data"][0]["expires_at"], due);
            // Link ở `ledger` (và ở đối tác) hết hạn đúng hạn của đơn, không muộn hơn.
            assert_eq!(json!(shop.ledger.last_payment().unwrap().expires_at), due);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quyet_dinh_da_bi_thay_doc_sau_khoa_don() {
    TestDatabase::run(async |test| {
        use crate::entity::payment;
        use autoxide_kernel::tenancy::ScopeExt as _;
        use sea_orm::sea_query::Expr;
        use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};

        let guests = guests(test, false).await;
        let shop = &guests.shop;
        let (order, _, _) = card_order(shop).await;
        let lot = shop.ledger.payments()[0].clone();
        // Khoản chuyển `superseded` ngay trước khi thông báo đã thu được xử lý. Không còn bước quyết định
        // nào ngoài khoá (`payment_succeeded` chỉ quyết trên dòng đọc sau `lifecycle::lock`), nên bài này
        // giữ đường rẽ `Recorded::Superseded` trong transaction; thứ tự với lần thay lượt do khoá dòng đơn
        // bảo đảm (cả hai bên đều khoá nó trước).
        payment::Entity::update_many()
            .col_expr(payment::Column::Status, Expr::value(crate::payment_attempts::SUPERSEDED))
            .filter(payment::Column::LedgerPaymentId.eq(lot.id))
            .scoped(&shop.default_tenant())
            .exec(shop.database().write())
            .await
            .unwrap();
        paid(shop, &lot).await;
        drain(shop).await;
        paid(shop, &lot).await;
        drain(shop).await;
        let boss = Call::as_user(shop, "boss");
        let (_, view) = boss.send("GET", &format!("/{order}"), None).await;
        assert_eq!(view["data"]["status"], "pending_payment", "{view}");
        assert_eq!(super::minor(&view["data"]["totals"]["paid"]), 0);
        assert_eq!(
            super::refund_rows(&shop.ledger),
            vec![(lot.id, lot.amount.minor(), "superseded_payment".to_owned())]
        );
        assert_eq!(superseded_events(shop).await, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_tac_tu_choi_hoan_luot_cu_thi_khong_thu_lai_mai() {
    TestDatabase::run(async |test| {
        let guests = guests(test, false).await;
        let shop = &guests.shop;
        let (order, number, _) = card_order(shop).await;
        let old = shop.ledger.payments()[0].clone();
        shop.ledger.fail(old.id);
        let token = access_token(&guests, &number).await;
        let uri = format!("/public/orders/{order}/payment-link");
        let (status, _, opened) =
            send(shop, &uri, &[("Order-Token", &token), ("Idempotency-Key", "x1")], json!({}))
                .await;
        assert_eq!(status, StatusCode::OK, "{opened}");
        // Lượt cũ báo đã thu, đối tác từ chối hoàn (4xx): event giao xong, không nằm chờ thử lại; người
        // vận hành thấy mục `late_payment` ở `ledger` (test ở module `ledger`).
        shop.ledger.refuse_next_refunds(1);
        paid(shop, &old).await;
        drain(shop).await;
        assert_eq!(undelivered(shop).await, 0);
        assert!(shop.ledger.refunds().is_empty());
        assert_eq!(superseded_events(shop).await, 1);
    })
    .await
    .unwrap();
}
