//! Trả một phần / toàn bộ đơn bằng tender (điểm thưởng, thẻ quà tặng — bước 47) cắm qua
//! `OrderExtensions::add_tender`: phần còn lại thu qua đối tác, trả đủ thì đơn xác nhận ngay, tender
//! lạ hay tender lỗi thì mọi chỗ giữ được trả.

use autoxide_kernel::time::Timestamp;
use std::sync::{Arc, Mutex};

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::checkout::{body, checkout, ready_cart};
use super::{Call, shop};
use crate::api::{OrderExtensions, Tender, TenderLimit};

/// Tender giả: giữ `input.amount` (tối đa trần), hay báo lỗi khi `input.fail`.
#[derive(Default)]
struct FakeTender {
    held: Mutex<Vec<(String, i64)>>,
    released: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Tender for FakeTender {
    fn kind(&self) -> &'static str {
        "points"
    }

    async fn hold(
        &self,
        _tenant: &TenantContext,
        reference: &str,
        buyer: Option<Uuid>,
        input: &Value,
        max: Money,
        _expires_at: Timestamp,
    ) -> Result<Money, Error> {
        assert!(buyer.is_some());
        if input["fail"] == true {
            return Err(Error::new(
                ErrorKind::Conflict,
                "orders.quote_changed",
                "Not enough points.",
            ));
        }
        let amount = input["amount"].as_i64().unwrap().min(max.minor());
        self.held.lock().unwrap().push((reference.to_owned(), amount));
        Ok(Money::new(amount, max.currency()))
    }

    async fn release(&self, _tenant: &TenantContext, reference: &str) -> Result<(), Error> {
        self.released.lock().unwrap().push(reference.to_owned());
        Ok(())
    }

    /// Báo giá giỏ: tối đa nửa phần phải trả, 1 điểm = 1.000 đ; khách vãng lai không báo.
    async fn limit(
        &self,
        _tenant: &TenantContext,
        buyer: Option<Uuid>,
        max: Money,
    ) -> Result<Option<TenderLimit>, Error> {
        let amount = max.minor() / 2;
        Ok(buyer.map(|_| TenderLimit::new(Money::new(amount, max.currency()), Some(amount / 1000))))
    }
}

#[tokio::test]
async fn tender_pays_part_or_all_of_the_order() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let tender = Arc::new(FakeTender::default());
        shop.api::<dyn OrderExtensions>().unwrap().add_tender(tender.clone()).unwrap();
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 5);
        let buyer = Call::as_user(&shop, "buyer1");

        // Một phần: 50.000 bằng điểm, phần còn lại (130.000 − 50.000) qua đối tác.
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        // Báo giá giỏ cho biết tối đa trả được bằng điểm (nửa của 130.000).
        let (_, read) = buyer.send("GET", &format!("/carts/{cart}"), None).await;
        assert_eq!(
            read["data"]["tender_limits"],
            json!([{"kind": "points", "max_amount": {"amount": "65000", "currency": "VND"},
                "max_units": 65}]),
            "{read}"
        );
        let mut request = body(&cart, &quote, "card");
        request["tenders"] = json!([{"kind": "points", "input": {"amount": 50000}}]);
        let (status, placed) = checkout(&buyer, Some("t-1"), request).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        let order = &placed["data"]["orders"][0];
        assert_eq!(order["status"], "pending_payment");
        let id = order["id"].as_str().unwrap().to_owned();
        let payment = shop.ledger.payments.lock().unwrap()[0].clone();
        assert_eq!(payment.amount.minor(), 80_000);
        assert_eq!(tender.held.lock().unwrap()[0], (format!("orders:{id}"), 50_000));
        let (_, mine) = buyer.send("GET", &format!("/me/orders/{id}"), None).await;
        assert_eq!(mine["data"]["payment_status"], "partially_paid", "{mine}");

        // Toàn bộ: điểm phủ hết → xác nhận ngay, không lượt thu nào ở đối tác.
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;
        let mut request = body(&cart, &quote, "card");
        request["tenders"] = json!([{"kind": "points", "input": {"amount": 10_000_000}}]);
        let (status, placed) = checkout(&buyer, Some("t-2"), request).await;
        assert_eq!(status, StatusCode::CREATED, "{placed}");
        assert_eq!(placed["data"]["orders"][0]["status"], "confirmed");
        assert_eq!(shop.ledger.payments.lock().unwrap().len(), 1);
        assert_eq!(tender.held.lock().unwrap()[1].1, 130_000);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn unknown_or_failing_tender_releases_every_hold() {
    TestDatabase::run(async |test| {
        let shop = shop(test).await;
        let tender = Arc::new(FakeTender::default());
        shop.api::<dyn OrderExtensions>().unwrap().add_tender(tender.clone()).unwrap();
        let shirt = shop.catalog.add(shop.tree.a, 100_000);
        shop.inventory.stock(shop.tree.a, shirt, 1);
        let buyer = Call::as_user(&shop, "buyer1");
        let (cart, _, quote) = ready_cart(&buyer, shop.tree.a, shirt, 1).await;

        let mut request = body(&cart, &quote, "card");
        request["tenders"] = json!([{"kind": "gift_card", "input": {}}]);
        let (status, problem) = checkout(&buyer, Some("u-1"), request).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("orders.tender_unavailable"))
        );
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 1);

        let mut request = body(&cart, &quote, "card");
        request["tenders"] = json!([{"kind": "points", "input": {"fail": true}}]);
        let (status, problem) = checkout(&buyer, Some("u-2"), request).await;
        assert_eq!(
            (status, problem["code"].as_str()),
            (StatusCode::CONFLICT, Some("orders.quote_changed"))
        );
        assert_eq!(shop.inventory.available(shop.tree.a, shirt), 1);
        assert_eq!(tender.released.lock().unwrap().len(), 0);
    })
    .await
    .unwrap();
}
