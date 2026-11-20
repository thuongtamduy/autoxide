//! Hoá đơn điện tử của đơn theo `invoice_timing`, hoá đơn điều chỉnh khi trả hàng, chứng từ của
//! người mua, thư xác nhận / trạng thái qua `notifications`, và bán tại quầy (ca, bán, đóng ca có
//! chênh lệch, duyệt).

use std::sync::{Arc, Mutex};

use autoxide_invoices::api::{
    Buyer, Document, Invoices, NewDocument, NewDocumentLine, RenderFormat, Rendered,
};
use autoxide_kernel::access::{Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use autoxide_ledger::api::AccountKey;
use autoxide_notifications::api::{Notification, NotificationKind, Notifier, Outcome, Recipient};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use futures_util::future::join_all;
use serde_json::{Value, json};
use uuid::Uuid;

use super::lifecycle::{cod_order, count_events, status_of};
use super::returns::{delivered_order, return_body, step};
use super::{Call, Shop, drain, minor, money, shop_with, vnd};

/// `invoices` giả: giữ chứng từ đã lập (nháp → phát hành), lập điều chỉnh, in PDF.
#[derive(Default)]
struct FakeInvoices {
    /// (chứng từ, nháp đã nhận); điều chỉnh: `relation = adjustment`.
    documents: Mutex<Vec<(Document, NewDocument)>>,
}

fn document(draft: &NewDocument, relation: Option<(&str, Uuid, &str)>) -> Document {
    let total: i64 = draft
        .lines
        .iter()
        .map(|line| {
            let unit = line.unit_price.map_or(0, |price| price.minor());
            let quantity: i64 = line
                .quantity
                .as_ref()
                .map_or(1, |quantity| quantity.0.to_string().parse().unwrap_or(1));
            unit * quantity - line.discount.map_or(0, |discount| discount.minor())
        })
        .sum();
    let at = chrono::Utc::now();
    Document {
        id: Uuid::now_v7(),
        org_id: draft.org.as_uuid(),
        on_behalf_of_org_id: None,
        kind: draft.kind.clone(),
        relation: relation.map(|(relation, _, _)| relation.to_owned()),
        related_document_id: relation.map(|(_, original, _)| original),
        direction: relation.map_or("increase", |(_, _, direction)| direction).to_owned(),
        series_id: None,
        symbol: None,
        number: None,
        status: "draft".to_owned(),
        issue_date: None,
        prices_include_tax: draft.prices_include_tax,
        currency: "VND".to_owned(),
        subtotal: money(total),
        tax: money(0),
        total: money(total),
        buyer: draft.buyer.clone(),
        seller: json!({}),
        lines: Vec::new(),
        einvoice_provider: None,
        provider_ref: None,
        provider_number: None,
        authority_code: None,
        lookup_code: None,
        rejection_reason: None,
        retention_until: None,
        exchange_rate: None,
        exchange_rate_id: None,
        base_total: None,
        source_module: draft.source_module.clone(),
        source_id: draft.source_id.clone(),
        reference: draft.reference.clone(),
        issued_at: None,
        created_at: at,
        updated_at: at,
    }
}

#[async_trait::async_trait]
impl Invoices for FakeInvoices {
    async fn create_document(
        &self,
        _: &TenantContext,
        _: Actor,
        draft: NewDocument,
    ) -> Result<Submitted<Document>, Error> {
        let mut documents = self.documents.lock().unwrap();
        if let Some((found, _)) =
            documents.iter().find(|(found, _)| found.reference == draft.reference)
        {
            return Ok(Submitted::Replayed(found.clone()));
        }
        let made = document(&draft, None);
        documents.push((made.clone(), draft));
        Ok(Submitted::Created(made))
    }

    async fn issue(&self, _: &TenantContext, _: Actor, id: Uuid) -> Result<Document, Error> {
        let mut documents = self.documents.lock().unwrap();
        let count = documents.iter().filter(|(found, _)| found.number.is_some()).count();
        let (found, _) = documents.iter_mut().find(|(found, _)| found.id == id).unwrap();
        if found.number.is_none() {
            found.number = Some(count as i64 + 1);
            found.status = "issued".to_owned();
            found.symbol = Some("1C26TAA".to_owned());
        }
        Ok(found.clone())
    }

    async fn document(&self, _: &TenantContext, id: Uuid) -> Result<Option<Document>, Error> {
        let documents = self.documents.lock().unwrap();
        Ok(documents.iter().find(|(found, _)| found.id == id).map(|(found, _)| found.clone()))
    }

    async fn adjust(
        &self,
        _: &TenantContext,
        _: Actor,
        original: Uuid,
        direction: &str,
        lines: Vec<NewDocumentLine>,
        reference: String,
    ) -> Result<Submitted<Document>, Error> {
        let mut documents = self.documents.lock().unwrap();
        if let Some((found, _)) = documents.iter().find(|(found, _)| found.reference == reference) {
            return Ok(Submitted::Replayed(found.clone()));
        }
        let (_, base) = documents.iter().find(|(found, _)| found.id == original).unwrap().clone();
        let mut draft = NewDocument::new(
            base.org,
            base.kind.clone(),
            vnd(),
            base.buyer.clone(),
            lines,
            "invoices",
            reference,
        );
        draft.booked_by_source = base.booked_by_source;
        let made = document(&draft, Some(("adjustment", original, direction)));
        documents.push((made.clone(), draft));
        Ok(Submitted::Created(made))
    }

    async fn cancel(
        &self,
        _: &TenantContext,
        _: Actor,
        _: Uuid,
        _: String,
    ) -> Result<Document, Error> {
        Err(Error::internal("not used by orders"))
    }

    async fn render(
        &self,
        _: &TenantContext,
        id: Uuid,
        _: RenderFormat,
        _: &str,
    ) -> Result<Rendered, Error> {
        Ok(Rendered::new("application/pdf", format!("{id}.pdf"), b"%PDF-1.7 fake".to_vec()))
    }

    async fn archive(&self, _: &TenantContext, _: Actor) -> Result<Uuid, Error> {
        Err(Error::internal("not used by orders"))
    }
}

/// `notifications` giả: giữ loại đã khai và tin đã gửi (cùng `dedupe_key` → không thêm).
#[derive(Default)]
struct FakeNotifier {
    kinds: Mutex<Vec<&'static str>>,
    sent: Mutex<Vec<Notification>>,
}

#[async_trait::async_trait]
impl Notifier for FakeNotifier {
    fn declare(&self, kind: NotificationKind) -> Result<(), Error> {
        self.kinds.lock().unwrap().push(kind.key());
        Ok(())
    }

    async fn notify(&self, _: TenantId, notification: Notification) -> Result<Outcome, Error> {
        let mut sent = self.sent.lock().unwrap();
        if !sent.iter().any(|old| old.dedupe_key == notification.dedupe_key) {
            sent.push(notification);
        }
        Ok(Outcome::default())
    }
}

struct World {
    shop: Shop,
    invoices: Arc<FakeInvoices>,
    notifier: Arc<FakeNotifier>,
}

async fn world(test: &TestDatabase) -> World {
    let invoices = Arc::new(FakeInvoices::default());
    let notifier = Arc::new(FakeNotifier::default());
    let (invoices_in, notifier_in) = (invoices.clone(), notifier.clone());
    let shop = shop_with(test, move |builder, tree| {
        builder
            .provide::<dyn Invoices>(invoices_in)
            .provide::<dyn Notifier>(notifier_in)
            .principal("boss2", Principal::with_all_permissions(Uuid::from_u128(3)))
            .member("cashier_a", 4, tree.a, &[("orders.pos.sell", Scope::Unit)])
    })
    .await;
    World { shop, invoices, notifier }
}

fn kinds_of(world: &World) -> Vec<String> {
    world.invoices.documents.lock().unwrap().iter().map(|(found, _)| found.kind.clone()).collect()
}

#[tokio::test]
async fn hoa_don_lap_khi_giao_xong_va_dieu_chinh_khi_tra_hang() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        let shop = &world.shop;
        // Chưa giao: chưa có hoá đơn (mặc định `on_delivered`).
        cod_order(shop).await;
        drain(shop).await;
        assert!(kinds_of(&world).is_empty());
        let (id, shirt) = delivered_order(shop).await;
        drain(shop).await;
        {
            let documents = world.invoices.documents.lock().unwrap();
            assert_eq!(documents.len(), 1);
            let (found, draft) = &documents[0];
            assert_eq!(found.kind, "vat_invoice");
            assert_eq!(found.status, "issued");
            // Doanh thu do `orders` đã ghi: hoá đơn không ghi lần hai.
            assert!(draft.booked_by_source);
            assert!(draft.prices_include_tax);
            assert_eq!(draft.reference, format!("orders:{id}:invoice"));
            // Áo + dòng phí giao.
            assert_eq!(draft.lines.len(), 2);
            assert_eq!(draft.buyer.kind, "consumer");
        }
        assert_eq!(count_events(shop, "orders.invoice_linked").await, 1);
        let buyer = Call::as_user(shop, "buyer1");
        let (status, docs) = buyer.send("GET", &format!("/me/orders/{id}/documents"), None).await;
        assert_eq!(status, StatusCode::OK, "{docs}");
        assert_eq!(docs["data"].as_array().unwrap().len(), 1);
        assert_eq!(docs["data"][0]["kind"], "sales");
        assert_eq!(docs["data"][0]["number"], 1);
        let document = docs["data"][0]["document_id"].as_str().unwrap().to_owned();
        let answer = shop
            .get(&format!("/api/v1/orders/me/orders/{id}/documents/{document}"))
            .token("buyer1")
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::OK);
        assert_eq!(answer.header("content-type"), "application/pdf");
        // Người khác không xem được.
        let (status, _) = Call::as_user(shop, "buyer2")
            .send("GET", &format!("/me/orders/{id}/documents"), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Đã có hoá đơn: xin lại → 409.
        let (status, again) =
            buyer.send("POST", &format!("/me/orders/{id}/invoice"), Some(json!({}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{again}");
        assert_eq!(again["code"], "orders.invoice_already_issued");

        // Trả 1 áo → hoàn tiền → hoá đơn điều chỉnh giảm đúng 1 đơn vị.
        shop.orgs.set(shop.tree.a, "orders.allowed_return_reasons", json!(["changed_mind"]));
        let (status, asked) = buyer
            .send(
                "POST",
                &format!("/me/orders/{id}/returns"),
                Some(return_body(shirt, 1, "changed_mind")),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{asked}");
        let return_id = asked["data"]["id"].as_str().unwrap().to_owned();
        for action in ["approve", "receive", "refund"] {
            let (status, moved) = step(shop, &return_id, json!({"action": action})).await;
            assert_eq!(status, StatusCode::OK, "{action}: {moved}");
        }
        drain(shop).await;
        drain(shop).await;
        {
            let documents = world.invoices.documents.lock().unwrap();
            assert_eq!(
                documents.len(),
                2,
                "{:?}",
                documents.iter().map(|d| &d.0.reference).collect::<Vec<_>>()
            );
            let (adjustment, draft) = &documents[1];
            assert_eq!(adjustment.relation.as_deref(), Some("adjustment"));
            assert_eq!(adjustment.direction, "decrease");
            assert_eq!(adjustment.related_document_id, Some(documents[0].0.id));
            assert_eq!(draft.lines.len(), 1);
            assert_eq!(draft.lines[0].unit_price.map(|price| price.minor()), Some(100_000));
        }
        let (_, docs) = buyer.send("GET", &format!("/me/orders/{id}/documents"), None).await;
        assert_eq!(docs["data"].as_array().unwrap().len(), 2);
        assert_eq!(docs["data"][1]["kind"], "adjustment");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hoa_don_theo_yeu_cau_cho_doanh_nghiep() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        let shop = &world.shop;
        shop.orgs.set(shop.tree.a, "orders.invoice_timing", json!("on_request"));
        let (id, _) = delivered_order(shop).await;
        drain(shop).await;
        assert!(kinds_of(&world).is_empty(), "on_request: chưa xin thì chưa lập");
        let buyer = Call::as_user(shop, "buyer1");
        let uri = format!("/me/orders/{id}/invoice");
        let (status, refused) = buyer.send("POST", &uri, Some(json!({"tax_code": "12ab"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        let (status, issued) = buyer
            .send(
                "POST",
                &uri,
                Some(json!({"tax_code": "0101234567", "name": "Công ty An Bình",
                    "address": "1 Tràng Tiền, Hà Nội", "email": "ketoan@example.com"})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{issued}");
        assert_eq!(issued["data"]["document"]["status"], "issued");
        let documents = world.invoices.documents.lock().unwrap();
        let buyer: &Buyer = &documents[0].1.buyer;
        assert_eq!(buyer.kind, "business");
        assert_eq!(buyer.tax_code.as_deref(), Some("0101234567"));
        assert_eq!(buyer.email.as_deref(), Some("ketoan@example.com"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thu_xac_nhan_va_trang_thai_gui_mot_lan() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        let shop = &world.shop;
        assert!(world.notifier.kinds.lock().unwrap().contains(&"orders.order_placed"));
        let (id, _) = delivered_order(shop).await;
        drain(shop).await;
        drain(shop).await;
        let sent = world.notifier.sent.lock().unwrap().clone();
        let kinds: Vec<&str> = sent.iter().map(|notification| notification.kind.as_str()).collect();
        assert!(kinds.contains(&"orders.order_placed"), "{kinds:?}");
        assert!(kinds.contains(&"orders.order_delivered"), "{kinds:?}");
        // COD: không có thư "đã thanh toán".
        assert!(!kinds.contains(&"orders.order_confirmed"), "{kinds:?}");
        let placed = sent.iter().find(|n| n.kind == "orders.order_placed").unwrap();
        assert_eq!(placed.recipient, Recipient::User(Uuid::from_u128(101)));
        let (_, found) = Call::as_user(shop, "boss").send("GET", &format!("/{id}"), None).await;
        assert_eq!(placed.params["number"], found["data"]["number"]);
        // Mỗi event một tin: chạy lại không gửi thêm.
        let before = sent.len();
        drain(shop).await;
        assert_eq!(world.notifier.sent.lock().unwrap().len(), before);
    })
    .await
    .unwrap();
}

/// Mở ca của quầy `Q1` cửa hàng A với 500.000 tiền đầu ca; trả ID ca.
async fn open_shift(shop: &Shop, who: &str) -> String {
    let (status, opened) = Call::as_user(shop, who)
        .send(
            "POST",
            "/pos/shifts",
            Some(json!({"org_id": shop.tree.a.as_uuid(), "register_code": "Q1",
                "opening_cash": {"amount": "500000", "currency": "VND"}})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    opened["data"]["id"].as_str().unwrap().to_owned()
}

fn vnd_json(minor: i64) -> Value {
    json!({"amount": minor.to_string(), "currency": "VND"})
}

#[tokio::test]
async fn ban_tai_quay_tru_kho_cua_hang_va_gui_lai_cung_ma() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        let shop = &world.shop;
        let tea = shop.catalog.add(shop.tree.a, 45_000);
        shop.inventory.stock(shop.tree.a, tea, 3);
        let shift = open_shift(shop, "boss").await;
        // Một ca mở mỗi quầy.
        let (status, twice) = Call::as_user(shop, "boss")
            .send(
                "POST",
                "/pos/shifts",
                Some(json!({"org_id": shop.tree.a.as_uuid(), "register_code": "Q1",
                    "opening_cash": vnd_json(0)})),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{twice}");
        assert_eq!(twice["code"], "orders.shift_already_open");

        let cashier = Call::as_user(shop, "cashier_a");
        let lines = json!([{"variant_id": tea, "quantity": 2}]);
        let (status, quoted) = cashier
            .send("POST", "/pos/quote", Some(json!({"shift_id": shift, "lines": lines, "codes": ["NOPE"]})))
            .await;
        assert_eq!(status, StatusCode::OK, "{quoted}");
        assert_eq!(minor(&quoted["data"]["totals"]["total"]), 90_000);

        // Tổng khoản thu khác tổng đơn: 422.
        let sale = |payments: Value| {
            json!({"shift_id": shift, "reference": "R-1", "lines": lines, "payments": payments})
        };
        let (status, mismatch) = cashier
            .send("POST", "/pos/sales", Some(sale(json!([{"method": "cash", "amount": vnd_json(50_000)}]))))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{mismatch}");
        assert_eq!(mismatch["code"], "orders.payments_mismatch");

        let paid = json!([{"method": "cash", "amount": vnd_json(60_000), "tendered": vnd_json(100_000)},
            {"method": "card_terminal", "amount": vnd_json(30_000), "approval_code": "A1"}]);
        let (status, sold) = cashier.send("POST", "/pos/sales", Some(sale(paid.clone()))).await;
        assert_eq!(status, StatusCode::CREATED, "{sold}");
        assert_eq!(sold["data"]["order"]["status"], "delivered");
        assert_eq!(sold["data"]["order"]["channel"], "pos");
        assert_eq!(sold["data"]["order"]["payment_status"], "paid");
        assert_eq!(minor(&sold["data"]["change"]), 40_000);
        let order = sold["data"]["order"]["id"].as_str().unwrap().to_owned();
        assert_eq!(shop.inventory.available(shop.tree.a, tea), 1);

        // Máy quầy gửi lại cùng mã (kể cả song song): cùng một đơn, không trừ kho thêm.
        let again = (0..6).map(|_| cashier.send("POST", "/pos/sales", Some(sale(paid.clone()))));
        for (status, replay) in join_all(again).await {
            assert_eq!(status, StatusCode::OK, "{replay}");
            assert_eq!(replay["data"]["order"]["id"], order.as_str());
            assert_eq!(minor(&replay["data"]["change"]), 40_000);
        }
        assert_eq!(shop.inventory.available(shop.tree.a, tea), 1);
        // Cùng mã, nội dung khác: 422.
        let other = json!([{"method": "cash", "amount": vnd_json(90_000)}]);
        let (status, reused) = cashier.send("POST", "/pos/sales", Some(sale(other))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{reused}");
        assert_eq!(reused["code"], "orders.idempotency_key_reused");

        // Hoá đơn khởi tạo từ máy tính tiền khi giao (khách nhận tại chỗ).
        drain(shop).await;
        assert_eq!(kinds_of(&world), vec!["cash_register_invoice".to_owned()]);
        // Sổ: tiền mặt vào két, thẻ vào tài khoản chờ của ngân hàng.
        let cash = AccountKey::new(shop.tree.a, "cash.on_hand", vnd());
        assert_eq!(shop.ledger.balance_of(&cash), 60_000);
        assert_eq!(status_of(shop, order.parse().unwrap()).await, "delivered");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dong_ca_co_chenh_lech_can_ly_do_ghi_so_va_duyet_khi_vuot_nguong() {
    TestDatabase::run(async |test| {
        let world = world(test).await;
        let shop = &world.shop;
        let tea = shop.catalog.add(shop.tree.a, 45_000);
        shop.inventory.stock(shop.tree.a, tea, 10);
        let shift = open_shift(shop, "boss").await;
        let sale = json!({"shift_id": shift, "reference": "R-1",
            "lines": [{"variant_id": tea, "quantity": 1}],
            "payments": [{"method": "cash", "amount": vnd_json(45_000), "tendered": vnd_json(50_000)}]});
        let (status, sold) = Call::as_user(shop, "cashier_a").send("POST", "/pos/sales", Some(sale)).await;
        assert_eq!(status, StatusCode::CREATED, "{sold}");
        let boss = Call::as_user(shop, "boss");
        let close = format!("/pos/shifts/{shift}/close");
        // Dự kiến 545.000; đếm được 540.000: thiếu 5.000 → cần lý do và ghi chú.
        let (status, refused) =
            boss.send("POST", &close, Some(json!({"counted_cash": vnd_json(540_000)}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        let fields: Vec<&str> =
            refused["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap()).collect();
        assert!(fields.contains(&"variance_reason") && fields.contains(&"variance_note"), "{refused}");
        let (status, closed) = boss
            .send(
                "POST",
                &close,
                Some(json!({"counted_cash": vnd_json(540_000), "variance_reason": "wrong_change",
                    "variance_note": "Thối nhầm"})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{closed}");
        assert_eq!(closed["data"]["status"], "closed");
        assert_eq!(minor(&closed["data"]["expected_cash"]), 545_000);
        assert_eq!(minor(&closed["data"]["variance"]), -5_000);
        let shortage = AccountKey::new(shop.tree.a, "orders.cash_shortage", vnd());
        assert_eq!(shop.ledger.balance_of(&shortage), 5_000);
        assert_eq!(count_events(shop, "orders.pos_shift_closed").await, 1);
        // Đã đóng: không đóng lại, không bán tiếp.
        let (status, _) = boss.send("POST", &close, Some(json!({"counted_cash": vnd_json(0)}))).await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Ca mới, thừa 300.000 (vượt ngưỡng mặc định 200.000): chờ người khác duyệt.
        let shift = open_shift(shop, "boss").await;
        let (status, pending) = boss
            .send(
                "POST",
                &format!("/pos/shifts/{shift}/close"),
                Some(json!({"counted_cash": vnd_json(800_000), "variance_reason": "unrecorded_sale",
                    "variance_note": "Bán chưa ghi"})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{pending}");
        assert_eq!(pending["data"]["status"], "pending_approval");
        let overage = AccountKey::new(shop.tree.a, "orders.cash_overage", vnd());
        assert_eq!(shop.ledger.balance_of(&overage), 0, "chưa duyệt thì chưa ghi sổ");
        let approve = format!("/pos/shifts/{shift}/approve");
        let (status, own) = boss.send("POST", &approve, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{own}");
        assert_eq!(own["code"], "orders.shift_approval_self");
        let (status, approved) = Call::as_user(shop, "boss2").send("POST", &approve, None).await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        assert_eq!(approved["data"]["status"], "closed");
        assert_eq!(shop.ledger.balance_of(&overage), -300_000);
        // Thu ngân không có `orders.pos.shift`.
        let (status, _) = Call::as_user(shop, "cashier_a")
            .send(
                "POST",
                "/pos/shifts",
                Some(json!({"org_id": shop.tree.a.as_uuid(), "register_code": "Q2",
                    "opening_cash": vnd_json(0)})),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}
