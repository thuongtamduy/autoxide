//! `Ledger` giả cho test của `plans`: chỉ để `plans` thấy có `dyn Ledger` (nghe
//! `ledger.payment_succeeded`) và để test ghi event đó vào outbox như `ledger` thật. Các hàm không
//! dùng.

use autoxide_kernel::time::Date;
use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_ledger::api::{
    AccountKey, Entry, ExchangeRate, FlowSettings, Hold, JournalLine, JournalQuery, Ledger,
    NewEntry, NewPayment, NewPayout, NewRefund, NewReversal, NewTransferPayment, OpenAccount,
    Payment, Payout, Refund, Settled, Settlement,
};
use uuid::Uuid;

pub(crate) struct FakeLedger;

fn unused<T>() -> Result<T, Error> {
    Err(Error::internal("not used by plans tests"))
}

#[async_trait::async_trait]
impl Ledger for FakeLedger {
    async fn post(&self, _: &TenantContext, _: Actor, _: NewEntry) -> Result<Submitted<Entry>, Error> {
        unused()
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
        unused()
    }
    async fn balance(
        &self,
        _: &TenantContext,
        _: AccountKey,
        _: Option<Date>,
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

/// Event `ledger.payment_succeeded` như `ledger` phát.
#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct PaymentSucceeded(pub serde_json::Value);

impl autoxide_kernel::event::Event for PaymentSucceeded {
    const NAME: &'static str = "ledger.payment_succeeded";
}

/// Module `ledger` giả: công bố [`FakeLedger`], khai event `ledger.payment_succeeded` và giữ
/// `Outbox` để test ghi event đó (đi đúng đường thật: outbox → dispatcher → bộ nghe của `plans`).
#[derive(Default)]
pub(crate) struct FakeLedgerModule {
    pub outbox: Arc<Mutex<Option<Outbox>>>,
}

impl autoxide_kernel::module::Module for FakeLedgerModule {
    fn name(&self) -> &'static str {
        "ledger"
    }

    fn description(&self) -> &'static str {
        "Fake ledger for the tests of plans."
    }

    fn migrations(&self) -> Vec<Box<dyn autoxide_kernel::migration::prelude::MigrationTrait>> {
        vec![
            autoxide_kernel::event::outbox_migration("m0001_create_events", "ledger"),
            autoxide_kernel::event::outbox_actor_migration("m0002_add_event_actor", "ledger"),
            autoxide_kernel::tenancy::migration::adopt(
                "m0003_tenancy_events",
                vec![autoxide_kernel::event::tenant_table("ledger")],
            ),
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", "ledger"),
        ]
    }

    fn events(&self) -> Vec<autoxide_kernel::event::EventInfo> {
        vec![autoxide_kernel::event::EventInfo::new(
            "ledger.payment_succeeded",
            "A payment was collected.",
        )]
    }

    fn start(
        &mut self,
        context: &mut autoxide_kernel::module::StartContext<'_>,
    ) -> Result<(), Error> {
        *self.outbox.lock().unwrap() = Some(context.outbox()?);
        context.provide::<dyn Ledger>(Arc::new(FakeLedger))
    }
}

/// Ghi `ledger.payment_succeeded` của kỳ `period` với số thực thu `amount` (chưa giao: cần chạy
/// dispatcher).
pub(crate) async fn succeed(
    outbox: &Mutex<Option<Outbox>>,
    tenant: &TenantContext,
    period: Uuid,
    amount: Money,
) {
    let outbox = outbox.lock().unwrap().clone().unwrap();
    let payload = serde_json::json!({
        "payment_id": Uuid::now_v7(), "source_module": "plans", "source_id": period.to_string(),
        "provider_ref": format!("ref-{}", amount.minor()), "amount": amount,
    });
    let transaction = tenant.database().begin_write().await.unwrap();
    outbox
        .record(&transaction, tenant, Actor::system(), None, &PaymentSucceeded(payload))
        .await
        .unwrap();
    transaction.commit().await.unwrap();
}
