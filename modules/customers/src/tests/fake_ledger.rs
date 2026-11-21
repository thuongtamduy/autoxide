//! `Ledger` giả cho test của `customers`: mở tài khoản theo mục đích, ghi bút toán (chống trùng theo
//! `reference`), tính số dư theo bên thường. Các hàm khác không dùng.

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::HashMap;
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
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct FakeLedger {
    /// Tài khoản: ID → (mục đích, bên thường).
    accounts: Mutex<HashMap<Uuid, (String, Side)>>,
    /// Số dư theo mục đích (theo bên thường).
    balances: Mutex<HashMap<String, i64>>,
    references: Mutex<Vec<String>>,
}

fn unused<T>() -> Result<T, Error> {
    Err(Error::internal("not used by customers tests"))
}

#[async_trait::async_trait]
impl Ledger for FakeLedger {
    async fn post(
        &self,
        _: &TenantContext,
        _: Actor,
        entry: NewEntry,
    ) -> Result<Submitted<Entry>, Error> {
        let done = Entry {
            id: Uuid::now_v7(),
            book_id: Uuid::nil(),
            org_id: entry.book.as_uuid(),
            period_id: Uuid::nil(),
            effective_date: Timestamp::now().date_utc(),
            posted_at: Timestamp::now(),
            description: entry.description.clone(),
            source_module: entry.source_module.clone(),
            source_kind: entry.source_kind.clone(),
            source_id: entry.source_id.clone(),
            reference: entry.reference.clone(),
            reverses_entry_id: None,
            reversed_by_entry_id: None,
            actor_user_id: None,
            lines: Vec::new(),
            sequence: None,
            sealed_at: None,
        };
        let mut references = self.references.lock().unwrap();
        if references.contains(&entry.reference) {
            return Ok(Submitted::Replayed(done));
        }
        let (debits, credits): (i64, i64) =
            entry.lines.iter().fold((0, 0), |(d, c), line| match line.side {
                Side::Debit => (d + line.amount.minor(), c),
                Side::Credit => (d, c + line.amount.minor()),
            });
        assert_eq!(debits, credits, "unbalanced entry {}", entry.reference);
        references.push(entry.reference.clone());
        let accounts = self.accounts.lock().unwrap();
        let mut balances = self.balances.lock().unwrap();
        for line in &entry.lines {
            let LineAccount::Id(id) = &line.account else {
                panic!("customers posts by account id")
            };
            let (purpose, normal) = accounts.get(id).cloned().unwrap();
            let signed =
                if line.side == normal { line.amount.minor() } else { -line.amount.minor() };
            *balances.entry(purpose).or_default() += signed;
        }
        Ok(Submitted::Created(done))
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
    async fn open_account(&self, _: &TenantContext, account: OpenAccount) -> Result<Uuid, Error> {
        let mut accounts = self.accounts.lock().unwrap();
        if let Some((id, _)) =
            accounts.iter().find(|(_, (purpose, _))| *purpose == account.key.purpose)
        {
            return Ok(*id);
        }
        let id = Uuid::now_v7();
        accounts
            .insert(id, (account.key.purpose.clone(), account.normal_side.unwrap_or(Side::Debit)));
        Ok(id)
    }
    async fn balance(
        &self,
        _: &TenantContext,
        key: AccountKey,
        _: Option<Date>,
    ) -> Result<Money, Error> {
        let balance = self.balances.lock().unwrap().get(&key.purpose).copied().unwrap_or(0);
        Ok(Money::new(balance, key.currency))
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
        _: &[AccountKey],
        _: Option<Date>,
    ) -> Result<Vec<Money>, Error> {
        unused()
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
        _: Date,
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
