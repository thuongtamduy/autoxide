//! Nhập hàng: nhà cung cấp (công nợ qua `ledger`), đơn đặt hàng nhận một phần, phiếu nhập ghi sổ
//! cái đúng một lần, ghi lại khi lỗi, trả hàng nhà cung cấp đảo bút toán.

use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_ledger::api::{
    AccountKey, Entry, ExchangeRate, FlowSettings, Hold, JournalLine, JournalQuery, Ledger,
    LineAccount, NewEntry, NewPayment, NewPayout, NewRefund, NewReversal, NewTransferPayment,
    OpenAccount, Payment, Payout, Refund, Settled, Settlement, Side,
};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use chrono::{NaiveDate, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Shop, assert_consistent, call, drain, level, shop_with};

fn unused<T>() -> Result<T, Error> {
    Err(Error::internal("not used by inventory"))
}

/// Một bút toán đã ghi: mã tham chiếu và các dòng (bên, tài khoản, số tiền).
#[derive(Debug, Clone)]
struct Posted {
    reference: String,
    lines: Vec<(Side, LineAccount, i64)>,
}

/// Sổ cái giả: nhớ bút toán (chống trùng theo mã tham chiếu), lỗi lần ghi kế khi được bảo.
#[derive(Default)]
struct FakeLedger {
    posted: Mutex<Vec<Posted>>,
    fail_next: Mutex<bool>,
}

impl FakeLedger {
    fn references(&self) -> Vec<String> {
        self.posted.lock().unwrap().iter().map(|post| post.reference.clone()).collect()
    }
}

fn entry(reference: &str) -> Entry {
    serde_json::from_value(json!({
        "id": Uuid::now_v7(), "book_id": Uuid::now_v7(), "org_id": Uuid::now_v7(), "period_id": Uuid::now_v7(),
        "effective_date": "2026-11-19", "posted_at": Utc::now(), "description": "", "source_module": "inventory",
        "source_kind": "receipt", "source_id": null, "reference": reference, "reverses_entry_id": null,
        "reversed_by_entry_id": null, "actor_user_id": null, "lines": [], "sequence": null, "sealed_at": null,
    }))
    .unwrap()
}

#[async_trait::async_trait]
impl Ledger for FakeLedger {
    async fn post(
        &self,
        _: &TenantContext,
        _: Actor,
        new: NewEntry,
    ) -> Result<Submitted<Entry>, Error> {
        if std::mem::take(&mut *self.fail_next.lock().unwrap()) {
            // Mã của module `ledger` (không phải của module này): dựng lúc chạy.
            let code = ["ledger", "period_closed"].join(".");
            return Err(Error::conflict(code, "The period is closed."));
        }
        let mut posted = self.posted.lock().unwrap();
        if posted.iter().any(|post| post.reference == new.reference) {
            return Ok(Submitted::Replayed(entry(&new.reference)));
        }
        let lines = new
            .lines
            .iter()
            .map(|line| (line.side, line.account.clone(), line.amount.minor()))
            .collect();
        posted.push(Posted { reference: new.reference.clone(), lines });
        Ok(Submitted::Created(entry(&new.reference)))
    }
    async fn reverse(
        &self,
        _: &TenantContext,
        _: Actor,
        _: Uuid,
        _: NewReversal,
    ) -> Result<Submitted<Entry>, Error> {
        unused()
    }
    async fn ensure_account(&self, _: &TenantContext, _: AccountKey) -> Result<Uuid, Error> {
        unused()
    }
    async fn open_account(&self, _: &TenantContext, _: OpenAccount) -> Result<Uuid, Error> {
        Ok(Uuid::from_u128(42))
    }
    async fn balance(
        &self,
        _: &TenantContext,
        _: AccountKey,
        _: Option<NaiveDate>,
    ) -> Result<Money, Error> {
        unused()
    }
    async fn entry(&self, _: &TenantContext, _: Uuid) -> Result<Option<Entry>, Error> {
        unused()
    }
    async fn create_payment(
        &self,
        _: &TenantContext,
        _: Actor,
        _: NewPayment,
    ) -> Result<Submitted<Payment>, Error> {
        unused()
    }
    async fn refund(
        &self,
        _: &TenantContext,
        _: Actor,
        _: NewRefund,
    ) -> Result<Submitted<Refund>, Error> {
        unused()
    }
    async fn payment(&self, _: &TenantContext, _: Uuid) -> Result<Option<Payment>, Error> {
        unused()
    }
    async fn balances(
        &self,
        _: &TenantContext,
        keys: &[AccountKey],
        _: Option<NaiveDate>,
    ) -> Result<Vec<Money>, Error> {
        let posted = self.posted.lock().unwrap();
        Ok(keys
            .iter()
            .map(|key| {
                let sum: i64 = posted
                    .iter()
                    .flat_map(|post| post.lines.iter())
                    .filter(|(_, account, _)| *account == LineAccount::Key(key.clone()))
                    .map(|(side, _, amount)| if *side == Side::Credit { *amount } else { -amount })
                    .sum();
                Money::new(sum, key.currency)
            })
            .collect())
    }
    async fn flow_settings(&self, _: &TenantContext, _: OrgId) -> Result<FlowSettings, Error> {
        unused()
    }
    async fn settle(
        &self,
        _: &TenantContext,
        _: Actor,
        _: Settlement,
    ) -> Result<Submitted<Settled>, Error> {
        unused()
    }
    async fn hold_status(
        &self,
        _: &TenantContext,
        _: &str,
        _: &str,
    ) -> Result<Option<Hold>, Error> {
        unused()
    }
    async fn create_payout(
        &self,
        _: &TenantContext,
        _: Actor,
        _: NewPayout,
    ) -> Result<Submitted<Payout>, Error> {
        unused()
    }
    async fn create_transfer_payment(
        &self,
        _: &TenantContext,
        _: Actor,
        _: NewTransferPayment,
    ) -> Result<Submitted<Payment>, Error> {
        unused()
    }
    async fn exchange_rate(
        &self,
        _: &TenantContext,
        _: Currency,
        _: Currency,
        _: NaiveDate,
    ) -> Result<Option<ExchangeRate>, Error> {
        unused()
    }
    async fn book_currency(&self, _: &TenantContext, _: OrgId) -> Result<Currency, Error> {
        unused()
    }
    async fn journal(
        &self,
        _: &TenantContext,
        _: JournalQuery,
    ) -> Result<Page<JournalLine>, Error> {
        unused()
    }
}

async fn shop_with_ledger(test: &TestDatabase) -> (Shop, Arc<FakeLedger>) {
    let ledger = Arc::new(FakeLedger::default());
    let provided = ledger.clone();
    let shop = shop_with(test, move |builder, _| builder.provide::<dyn Ledger>(provided)).await;
    (shop, ledger)
}

fn vnd(minor: i64) -> Value {
    json!({"amount": minor.to_string(), "currency": "VND"})
}

async fn supplier(app: &TestApp, code: &str) -> String {
    let (status, made) = call(
        app,
        "POST",
        "/suppliers",
        "boss",
        Some(json!({"code": code, "name": "Công ty Trà Việt", "tax_code": "0101234567"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    made["data"]["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn nhap_hang_mot_phan_theo_po_va_ghi_so_cai_mot_lan() {
    TestDatabase::run(async |test| {
        let (shop, ledger) = shop_with_ledger(test).await;
        let tea = shop.catalog.item();
        let a = shop.tree.a.as_uuid();
        let supplier = supplier(&shop, "TRA-VIET").await;
        let (status, problem) = call(&shop, "POST", "/suppliers", "boss", Some(json!({"code": "TRA-VIET", "name": "Khác"}))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("inventory.supplier_code_taken")));

        let order = json!({"org_id": a, "supplier_id": supplier, "lines": [{"variant_id": tea, "quantity": 10, "unit_cost": vnd(50_000)}]});
        let (status, po) = call(&shop, "POST", "/purchase-orders", "boss", Some(order)).await;
        assert_eq!(status, StatusCode::CREATED, "{po}");
        assert_eq!(po["data"]["total"], vnd(500_000));
        let id = po["data"]["id"].as_str().unwrap().to_owned();
        let (status, ordered) = call(&shop, "POST", &format!("/purchase-orders/{id}/order"), "boss", None).await;
        assert_eq!((status, &ordered["data"]["status"]), (StatusCode::OK, &json!("ordered")));
        assert_eq!(level(&shop, shop.tree.a, tea).await["incoming"], 10);

        // Nhận 4 rồi quá số còn lại.
        let receipt = |reference: &str, quantity: i64| {
            json!({"org_id": a, "purchase_order_id": id, "reference": reference,
                   "lines": [{"variant_id": tea, "quantity": quantity, "unit_cost": vnd(50_000)}]})
        };
        let (status, first) = call(&shop, "POST", "/receipts", "boss", Some(receipt("GRN-1", 4))).await;
        assert_eq!(status, StatusCode::CREATED, "{first}");
        assert_eq!(first["data"]["ledger_status"], "pending");
        assert!(first["data"]["number"].as_str().unwrap().starts_with("PN-"));
        let (status, again) = call(&shop, "POST", "/receipts", "boss", Some(receipt("GRN-1", 4))).await;
        assert_eq!((status, &again["data"]["id"]), (StatusCode::OK, &first["data"]["id"]));
        let (status, problem) = call(&shop, "POST", "/receipts", "boss", Some(receipt("GRN-2", 7))).await;
        assert_eq!((status, problem["code"].as_str()), (StatusCode::CONFLICT, Some("inventory.over_receipt")));
        let found = level(&shop, shop.tree.a, tea).await;
        assert_eq!((found["on_hand"].as_i64(), found["incoming"].as_i64()), (Some(4), Some(6)));
        assert_eq!(found["average_cost"], vnd(50_000));
        let (_, po) = call(&shop, "GET", &format!("/purchase-orders/{id}"), "boss", None).await;
        assert_eq!(po["data"]["status"], "partially_received");

        // Job ghi sổ: chạy hai lần, ledger chỉ nhận một mã tham chiếu.
        drain(&shop).await;
        drain(&shop).await;
        let receipt_id = first["data"]["id"].as_str().unwrap();
        assert_eq!(ledger.references(), vec![format!("inventory.receipt:{receipt_id}")]);
        let (_, posted) = call(&shop, "GET", &format!("/receipts/{receipt_id}"), "boss", None).await;
        assert_eq!(posted["data"]["ledger_status"], "posted");
        // Công nợ nhà cung cấp: 4 × 50 000.
        let (_, found) = call(&shop, "GET", &format!("/suppliers/{supplier}"), "boss", None).await;
        assert_eq!(found["data"]["payable"], vnd(200_000));

        // Đóng đơn: 6 chưa về bỏ khỏi `incoming`.
        let (status, closed) = call(&shop, "POST", &format!("/purchase-orders/{id}/close"), "boss", None).await;
        assert_eq!((status, &closed["data"]["status"]), (StatusCode::OK, &json!("closed")));
        assert_eq!(level(&shop, shop.tree.a, tea).await["incoming"], 0);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ghi_so_loi_thi_ghi_lai_va_tra_hang_dao_but_toan() {
    TestDatabase::run(async |test| {
        let (shop, ledger) = shop_with_ledger(test).await;
        let tea = shop.catalog.item();
        let a = shop.tree.a.as_uuid();
        let supplier = supplier(&shop, "S-2").await;
        *ledger.fail_next.lock().unwrap() = true;
        let body = json!({"org_id": a, "supplier_id": supplier, "reference": "GRN-9",
                          "lines": [{"variant_id": tea, "quantity": 5, "unit_cost": vnd(10_000)}]});
        let (status, made) = call(&shop, "POST", "/receipts", "boss", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let id = made["data"]["id"].as_str().unwrap().to_owned();
        drain(&shop).await;
        let (_, failed) = call(&shop, "GET", &format!("/receipts/{id}"), "boss", None).await;
        assert_eq!((failed["data"]["ledger_status"].as_str(), failed["data"]["ledger_error_code"].as_str()), (Some("failed"), Some("ledger.period_closed")));
        // Ghi lại.
        let (status, retried) = call(&shop, "POST", &format!("/receipts/{id}/retry-ledger"), "boss", None).await;
        assert_eq!((status, &retried["data"]["ledger_status"]), (StatusCode::OK, &json!("pending")));
        drain(&shop).await;
        let (_, posted) = call(&shop, "GET", &format!("/receipts/{id}"), "boss", None).await;
        assert_eq!(posted["data"]["ledger_status"], "posted");

        // Trả 2 cho nhà cung cấp: giá vốn hiện tại, bút toán đảo (Nợ phải trả / Có hàng tồn kho).
        let back = json!({"org_id": a, "supplier_id": supplier, "receipt_id": id, "reference": "RET-1",
                          "lines": [{"variant_id": tea, "quantity": 2}]});
        let (status, returned) = call(&shop, "POST", "/supplier-returns", "boss", Some(back)).await;
        assert_eq!(status, StatusCode::CREATED, "{returned}");
        assert_eq!(returned["data"]["total"], vnd(20_000));
        assert!(returned["data"]["number"].as_str().unwrap().starts_with("TH-"));
        drain(&shop).await;
        assert_eq!(ledger.references().len(), 2);
        let posted = ledger.posted.lock().unwrap().last().cloned().unwrap();
        assert!(matches!(posted.lines[0], (Side::Debit, LineAccount::Key(_), 20_000)), "{posted:?}");
        let (_, found) = call(&shop, "GET", &format!("/suppliers/{supplier}"), "boss", None).await;
        assert_eq!(found["data"]["payable"], vnd(30_000));
        assert_eq!(level(&shop, shop.tree.a, tea).await["on_hand"], 3);
        let (_, ledger_lines) = call(&shop, "GET", "/movements?kind=supplier_return", "boss", None).await;
        assert_eq!(ledger_lines["data"][0]["quantity"], -2);
        assert_consistent(&shop).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_co_ledger_thi_khong_ghi_so() {
    TestDatabase::run(async |test| {
        let shop = super::shop(test).await;
        let tea = shop.catalog.item();
        let supplier = supplier(&shop, "S-3").await;
        let (_, found) = call(&shop, "GET", &format!("/suppliers/{supplier}"), "boss", None).await;
        assert!(found["data"].get("payable").is_none(), "{found}");
        let body = json!({"org_id": shop.tree.a.as_uuid(), "supplier_id": supplier, "reference": "GRN-X",
                          "lines": [{"variant_id": tea, "quantity": 1, "unit_cost": vnd(1)}]});
        let (_, made) = call(&shop, "POST", "/receipts", "boss", Some(body)).await;
        assert_eq!(made["data"]["ledger_status"], "not_applicable");
        // Nhân viên kho B không thấy nhà cung cấp của nút gốc? Nhà cung cấp ở gốc: keeper_b (độ sâu
        // `unit` ở B) không thấy.
        let (status, _) = call(&shop, "GET", &format!("/suppliers/{supplier}"), "keeper_b", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}
