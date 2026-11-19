//! Sổ cái giả cho test của `orders`: lượt thu (gửi lại cùng mã tham chiếu trả lượt cũ, hạn 30 phút),
//! hoàn tiền, cài đặt dòng tiền tự thu; bút toán nhớ theo mã tham chiếu, số dư bên Nợ trừ bên Có.

use std::sync::Mutex;

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
use chrono::{NaiveDate, Utc};
use uuid::Uuid;

fn unused<T>() -> Result<T, Error> {
    Err(Error::internal("not used by orders"))
}

/// ID tài khoản suy từ khoá (sổ, mục đích, đối tượng, tiền).
fn account_id(key: &AccountKey) -> Uuid {
    let party = key
        .counterparty
        .as_ref()
        .map(|party| format!("{}:{}", party.kind, party.id))
        .unwrap_or_default();
    let text = format!("{}|{}|{party}|{}", key.book, key.purpose, key.currency);
    let hash = |salt: &str| {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (salt, &text).hash(&mut hasher);
        hasher.finish()
    };
    Uuid::from_u64_pair(hash("a"), hash("b"))
}

/// Một bút toán đã ghi: mã tham chiếu và các dòng (tài khoản, bên, số tiền).
type Posted = (String, Vec<(Uuid, Side, i64)>);

#[derive(Default)]
pub(crate) struct FakeLedger {
    pub entries: Mutex<Vec<Posted>>,
    /// Lượt thu đã tạo, theo thứ tự.
    pub payments: Mutex<Vec<Payment>>,
    /// Lượt hoàn đã nhận: (ID thanh toán, số tiền, lý do).
    pub refunds: Mutex<Vec<(Uuid, i64, String)>>,
}

impl FakeLedger {
    fn payment_of(
        &self,
        org: OrgId,
        method: &str,
        amount: Money,
        reference: &str,
        qr: bool,
    ) -> Submitted<Payment> {
        let mut payments = self.payments.lock().unwrap();
        if let Some(found) = payments.iter().find(|payment| payment.reference == reference) {
            return Submitted::Replayed(found.clone());
        }
        let id = Uuid::now_v7();
        let made: Payment = serde_json::from_value(serde_json::json!({
            "id": id, "org_id": org.as_uuid(), "book_id": org.as_uuid(), "provider": "fake",
            "provider_ref": null, "method": method, "description": "Order", "amount": amount,
            "refunded": Money::zero(amount.currency()), "status": "pending",
            "checkout_url": if qr { None } else { Some(format!("https://pay.example/{id}")) },
            "qr_payload": if qr { Some(format!("000201-{id}")) } else { None },
            "transfer_code": if qr { Some("AX123") } else { None }, "bank_account_id": null,
            "expires_at": Utc::now() + chrono::Duration::minutes(30), "captured_at": null,
            "source_module": "orders", "source_id": null, "reference": reference,
            "created_at": Utc::now(), "updated_at": Utc::now(),
        }))
        .unwrap();
        payments.push(made.clone());
        Submitted::Created(made)
    }
}

impl FakeLedger {
    /// Số dư (Nợ − Có) của tài khoản `key`.
    pub(crate) fn balance_of(&self, key: &AccountKey) -> i64 {
        let id = account_id(key);
        self.entries
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(_, lines)| lines.iter())
            .filter(|(account, _, _)| *account == id)
            .map(|(_, side, amount)| if *side == Side::Debit { *amount } else { -amount })
            .sum()
    }
}

#[async_trait::async_trait]
impl Ledger for FakeLedger {
    async fn post(
        &self,
        _: &TenantContext,
        _: Actor,
        entry: NewEntry,
    ) -> Result<Submitted<Entry>, Error> {
        let mut entries = self.entries.lock().unwrap();
        let lines: Vec<(Uuid, Side, i64)> = entry
            .lines
            .iter()
            .map(|line| match &line.account {
                LineAccount::Id(id) => (*id, line.side, line.amount.minor()),
                LineAccount::Key(key) => (account_id(key), line.side, line.amount.minor()),
            })
            .collect();
        let replayed = entries.iter().any(|(reference, _)| reference == &entry.reference);
        if !replayed {
            entries.push((entry.reference.clone(), lines));
        }
        let made: Entry = serde_json::from_value(serde_json::json!({
            "id": Uuid::now_v7(), "book_id": entry.book.as_uuid(), "org_id": entry.book.as_uuid(),
            "period_id": Uuid::now_v7(), "effective_date": "2026-11-18", "posted_at": Utc::now(),
            "description": entry.description, "source_module": entry.source_module,
            "source_kind": entry.source_kind, "source_id": entry.source_id, "reference": entry.reference,
            "reverses_entry_id": null, "reversed_by_entry_id": null, "actor_user_id": null,
            "lines": [], "sequence": null, "sealed_at": null,
        }))
        .unwrap();
        Ok(if replayed { Submitted::Replayed(made) } else { Submitted::Created(made) })
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
    async fn ensure_account(&self, _: &TenantContext, key: AccountKey) -> Result<Uuid, Error> {
        Ok(account_id(&key))
    }
    async fn open_account(&self, _: &TenantContext, account: OpenAccount) -> Result<Uuid, Error> {
        Ok(account_id(&account.key))
    }
    async fn balance(
        &self,
        _: &TenantContext,
        key: AccountKey,
        _: Option<NaiveDate>,
    ) -> Result<Money, Error> {
        Ok(Money::new(self.balance_of(&key), key.currency))
    }
    async fn entry(&self, _: &TenantContext, _: Uuid) -> Result<Option<Entry>, Error> {
        unused()
    }
    async fn create_payment(
        &self,
        _: &TenantContext,
        _: Actor,
        payment: NewPayment,
    ) -> Result<Submitted<Payment>, Error> {
        Ok(self.payment_of(payment.org, &payment.method, payment.amount, &payment.reference, false))
    }
    async fn refund(
        &self,
        _: &TenantContext,
        _: Actor,
        refund: NewRefund,
    ) -> Result<Submitted<Refund>, Error> {
        self.refunds.lock().unwrap().push((
            refund.payment,
            refund.amount.minor(),
            refund.reason.clone(),
        ));
        Ok(Submitted::Created(
            serde_json::from_value(serde_json::json!({
                "id": Uuid::now_v7(), "payment_id": refund.payment, "amount": refund.amount,
                "status": "succeeded", "provider_ref": null, "reason": refund.reason,
                "reference": refund.reference, "created_at": Utc::now(), "updated_at": Utc::now(),
            }))
            .unwrap(),
        ))
    }
    async fn payment(&self, _: &TenantContext, _: Uuid) -> Result<Option<Payment>, Error> {
        unused()
    }
    async fn balances(
        &self,
        _: &TenantContext,
        _: &[AccountKey],
        _: Option<NaiveDate>,
    ) -> Result<Vec<Money>, Error> {
        unused()
    }
    async fn flow_settings(&self, _: &TenantContext, org: OrgId) -> Result<FlowSettings, Error> {
        Ok(serde_json::from_value(serde_json::json!({
            "org_id": org.as_uuid(), "source_org_id": null, "mode": "self_collect",
            "payment_provider": "fake", "payout_provider": null, "hold_days": 0,
            "cod_remittance": "partner_collection", "inherited": true, "updated_at": null,
        }))
        .unwrap())
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
        payment: NewTransferPayment,
    ) -> Result<Submitted<Payment>, Error> {
        Ok(self.payment_of(payment.org, "bank_transfer", payment.amount, &payment.reference, true))
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
