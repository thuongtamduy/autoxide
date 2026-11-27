//! Trả trước một phần cho hàng đặt trước (D-232): công bố trên giỏ, thu `upfront` lúc đặt, đơn
//! `partially_paid`, hàng về đủ → mời trả phần còn lại, chưa trả thì chưa giao, quá hạn → huỷ và giữ
//! lại theo điều kiện đã chụp, người mua huỷ trước mốc chốt → hoàn đủ.

use super::CatalogExt as _;
use autoxide_kernel::money::Currency;
use autoxide_kernel::time::Timestamp;
use autoxide_ledger::api::AccountKey;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use jiff::SignedDuration;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::{body, checkout};
use super::lifecycle::count_events;
use super::preorders::{cart_of, preorder_item, ship};
use super::{
    Call, PaymentSucceeded, PreorderAllocated, Shop, drain, emit, emit_from, minor, run_job, shop,
};
use crate::entity::{advance_plan, line};

/// Đặt bằng thẻ và báo `ledger` đã thu khoản vừa tạo; trả ID đơn và số tiền của khoản đó.
async fn place_paid(shop: &Shop, items: &[(Uuid, i64)]) -> (Uuid, i64) {
    let (cart, quote) = cart_of(shop, items).await;
    let mut request = body(&cart, &quote, "card");
    request["accept_preorder_terms"] = json!(true);
    let buyer = Call::as_user(shop, "buyer1");
    let (status, placed) = checkout(&buyer, Some(&Uuid::now_v7().to_string()), request).await;
    assert_eq!(status, StatusCode::CREATED, "{placed}");
    let id = placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap();
    let amount = pay_last(shop).await;
    (id, amount)
}

/// Báo `ledger` đã thu khoản mới nhất; trả số tiền.
async fn pay_last(shop: &Shop) -> i64 {
    let payment = shop.ledger.last_payment().unwrap();
    emit(
        shop,
        &PaymentSucceeded(
            json!({"payment_id": payment.id, "source_module": "orders", "amount": payment.amount}),
        ),
    )
    .await;
    drain(shop).await;
    payment.amount.minor()
}

/// Hàng đặt trước của đơn `id` đã về đủ.
async fn allocate(shop: &Shop, id: Uuid, variant: Uuid) {
    let row = line::Entity::find()
        .filter(line::Column::OrderId.eq(id))
        .filter(line::Column::VariantId.eq(variant))
        .one(shop.database().read())
        .await
        .unwrap()
        .unwrap();
    let allocated = json!({"after": {"order_id": id, "line_key": row.line_key, "variant_id": variant,
        "warehouse_org_id": shop.tree.a, "quantity": row.quantity, "allocated_quantity": row.quantity}});
    emit_from(shop, &shop.inventory_outbox, &PreorderAllocated(allocated)).await;
    drain(shop).await;
}

async fn mine(shop: &Shop, id: Uuid) -> Value {
    let (status, found) =
        Call::as_user(shop, "buyer1").send("GET", &format!("/me/orders/{id}"), None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    found["data"].clone()
}

#[tokio::test]
async fn tra_truoc_mot_phan_tu_luc_dat_toi_tra_phan_con_lai() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        shop.orgs.set(
            shop.tree.a,
            "orders.preorder_advance",
            json!({"rate_bp": 3000, "balance_due_hours": 48}),
        );
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 10);
        let phone = preorder_item(&shop, 300_000);

        // Công bố trước khi đặt: tổng 430.000 (gồm phí giao 30.000 chia theo tiền hàng: 7.500 /
        // 22.500) → phần đặt trước 322.500, trả trước 30% = 96.750.
        let (cart, _) = cart_of(&shop, &[(shirt, 1), (phone, 1)]).await;
        let (_, seen) =
            Call::as_user(&shop, "buyer1").send("GET", &format!("/carts/{cart}"), None).await;
        let advance = &seen["data"]["advance"];
        assert_eq!(minor(&advance["upfront"]), 204_250, "{advance}");
        assert_eq!(minor(&advance["balance"]), 225_750);
        assert_eq!(advance["balance_due_hours"], 48);
        assert_eq!(advance["lines"].as_array().unwrap().len(), 1);
        assert_eq!(minor(&advance["lines"][0]["amount"]), 96_750);

        let (id, paid) = place_paid(&shop, &[(shirt, 1), (phone, 1)]).await;
        assert_eq!(paid, 204_250, "lúc đặt chỉ thu hàng sẵn + phần trả trước");
        let order = mine(&shop, id).await;
        assert_eq!(order["status"], "confirmed");
        assert_eq!(order["payment_status"], "partially_paid");
        assert_eq!(order["advance"]["status"], "advance_paid");
        assert_eq!(minor(&order["advance"]["balance"]), 225_750);

        // Chưa có gì để trả thêm trước khi hàng về.
        let buyer = Call::as_user(&shop, "buyer1");
        let (status, problem) =
            buyer.send("POST", &format!("/me/orders/{id}/payments"), None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "orders.balance_not_requested");

        // Hàng sẵn đi trước.
        let (status, first) = ship(&shop, id).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{first}");

        // Hàng về → mời trả phần còn lại; event giao lại không mở link thứ hai.
        allocate(&shop, id, phone).await;
        allocate(&shop, id, phone).await;
        assert_eq!(count_events(&shop, "orders.preorder_balance_requested").await, 1);
        assert_eq!(shop.ledger.payments().len(), 2);
        let order = mine(&shop, id).await;
        assert_eq!(order["advance"]["status"], "balance_requested");
        assert!(order["advance"]["balance_due_at"].is_string());
        let (status, link) = buyer.send("POST", &format!("/me/orders/{id}/payments"), None).await;
        assert_eq!(status, StatusCode::OK, "{link}");
        assert_eq!(minor(&link["data"]["amount"]), 225_750);
        assert_eq!(link["data"]["kind"], "redirect");
        assert_eq!(shop.ledger.payments().len(), 2, "link còn hạn: dùng lại");

        // Trả xong → `paid`, hàng đặt trước giao được.
        assert_eq!(pay_last(&shop).await, 225_750);
        let order = mine(&shop, id).await;
        assert_eq!(order["payment_status"], "paid");
        assert_eq!(order["advance"]["status"], "balance_paid");
        assert_eq!(minor(&order["totals"]["paid"]), 430_000);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chua_tra_phan_con_lai_thi_chua_giao_qua_han_thi_huy_va_giu_lai() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        // Mốc chốt = ngày có hàng (sau 10 ngày) trừ 20 ngày: đã qua → giữ lại 50% phần trả trước.
        shop.orgs.set(
            shop.tree.a,
            "orders.preorder_advance",
            json!({"rate_bp": 3000, "commit_days": 20, "retention_bp": 5000}),
        );
        let phone = preorder_item(&shop, 300_000);
        // 300.000 + phí giao 30.000 → trả trước 99.000.
        let (id, paid) = place_paid(&shop, &[(phone, 1)]).await;
        assert_eq!(paid, 99_000);
        allocate(&shop, id, phone).await;
        let (status, problem) = ship(&shop, id).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "orders.balance_unpaid");

        // Quá hạn trả phần còn lại.
        advance_plan::Entity::update_many()
            .col_expr(
                advance_plan::Column::BalanceDueAt,
                Expr::value(Timestamp::now() - SignedDuration::from_hours(1)),
            )
            .filter(advance_plan::Column::OrderId.eq(id))
            .exec(shop.database().write())
            .await
            .unwrap();
        let (status, problem) = Call::as_user(&shop, "buyer1")
            .send("POST", &format!("/me/orders/{id}/payments"), None)
            .await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("orders.balance_overdue"))
        );
        run_job(&shop, crate::jobs::EXPIRE_PREORDER_BALANCES).await;
        run_job(&shop, crate::jobs::EXPIRE_PREORDER_BALANCES).await;
        assert_eq!(count_events(&shop, "orders.preorder_balance_overdue").await, 1);
        let order = mine(&shop, id).await;
        assert_eq!(order["status"], "cancelled");
        assert_eq!(order["advance"]["status"], "cancelled");
        let refunds = super::refund_rows(&shop.ledger);
        assert_eq!(refunds.len(), 1, "{refunds:?}");
        assert_eq!(refunds[0].1, 49_500, "hoàn phần trả trước trừ phần giữ lại");
        let income =
            AccountKey::new(shop.tree.a, crate::advance_cancel::CANCELLATION_INCOME, Currency::VND);
        assert_eq!(shop.ledger.net_debit(&income), -49_500, "Có thu nhập huỷ đơn");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_mua_huy_truoc_moc_chot_duoc_hoan_du() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        // Mốc chốt sau 9 ngày: huỷ bây giờ là trước mốc → hoàn đủ dù có tỉ lệ giữ lại.
        shop.orgs.set(
            shop.tree.a,
            "orders.preorder_advance",
            json!({"rate_bp": 3000, "commit_days": 1, "retention_bp": 5000}),
        );
        let phone = preorder_item(&shop, 300_000);
        let (id, paid) = place_paid(&shop, &[(phone, 1)]).await;
        assert_eq!(paid, 99_000);
        let (status, cancelled) = Call::as_user(&shop, "buyer1")
            .send("POST", &format!("/me/orders/{id}/cancel"), Some(json!({})))
            .await;
        assert_eq!(status, StatusCode::OK, "{cancelled}");
        drain(&shop).await;
        let refunds = super::refund_rows(&shop.ledger);
        assert_eq!(refunds.iter().map(|refund| refund.1).sum::<i64>(), 99_000);
        // Trả bằng COD: không trả trước một phần.
        let (cart, quote) = cart_of(&shop, &[(phone, 1)]).await;
        let mut request = body(&cart, &quote, "cod");
        request["accept_preorder_terms"] = json!(true);
        let (status, placed) =
            checkout(&Call::as_user(&shop, "buyer1"), Some("cod-1"), request).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        let cod = placed["data"]["orders"][0]["id"].as_str().unwrap().parse().unwrap();
        assert!(mine(&shop, cod).await.get("advance").is_none());
    })
    .await
    .unwrap();
}

/// UT-6 (rà #591): link trả phần còn lại hết hạn → mở link mới; link cũ bị thay (`superseded`). Đối tác
/// vẫn thu link cũ muộn: hoàn qua đối tác đúng một lần, không tính vào đơn.
#[tokio::test]
async fn link_phan_con_lai_bi_thay_thu_muon_thi_hoan() {
    TestDatabase::run(async |test| {
        use crate::entity::payment;
        let shop = shop(test).await;
        shop.orgs.set(
            shop.tree.a,
            "orders.preorder_advance",
            json!({"rate_bp": 3000, "balance_due_hours": 48}),
        );
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 10);
        let phone = preorder_item(&shop, 300_000);
        let (id, _) = place_paid(&shop, &[(shirt, 1), (phone, 1)]).await;
        let (status, first) = ship(&shop, id).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{first}");
        allocate(&shop, id, phone).await;
        let old = shop.ledger.last_payment().unwrap();

        // Link phần còn lại hết hạn → mở link mới.
        payment::Entity::update_many()
            .col_expr(
                payment::Column::ExpiresAt,
                Expr::value(Timestamp::now() - SignedDuration::from_mins(1)),
            )
            .filter(payment::Column::OrderId.eq(id))
            .filter(payment::Column::Purpose.eq(crate::advance::PURPOSE_BALANCE))
            .exec(shop.database().write())
            .await
            .unwrap();
        let buyer = Call::as_user(&shop, "buyer1");
        let (status, link) = buyer.send("POST", &format!("/me/orders/{id}/payments"), None).await;
        assert_eq!(status, StatusCode::OK, "{link}");
        let new = shop.ledger.last_payment().unwrap();
        assert_ne!(new.id, old.id);
        assert_eq!(pay_last(&shop).await, 225_750);
        assert_eq!(minor(&mine(&shop, id).await["totals"]["paid"]), 430_000);

        // Link cũ vẫn được thu (hai lần giao): hoàn đúng một lần, đơn không cộng thêm.
        for _ in 0..2 {
            shop.ledger.collect(old.id);
            emit(
                &shop,
                &PaymentSucceeded(
                    json!({"payment_id": old.id, "source_module": "orders", "amount": old.amount}),
                ),
            )
            .await;
            drain(&shop).await;
        }
        assert_eq!(
            super::refund_rows(&shop.ledger),
            vec![(old.id, old.amount.minor(), "superseded_payment".to_owned())]
        );
        assert_eq!(minor(&mine(&shop, id).await["totals"]["paid"]), 430_000);
    })
    .await
    .unwrap();
}
