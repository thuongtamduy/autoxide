//! Bộ giả cho test: sổ cái chỉ có thu và hoàn (đủ cho cọc), bộ gửi thông báo đếm tin.

use autoxide_kernel::time::{Date, Timestamp};
use jiff::SignedDuration;
use std::sync::Mutex;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
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

fn unused<T>() -> Result<T, Error> {
    Err(Error::internal("not used by bookings"))
}

/// Sổ cái giả: nhớ các lượt thu và hoàn.
#[derive(Default)]
pub(crate) struct FakeLedger {
    pub payments: Mutex<Vec<Payment>>,
    pub refunds: Mutex<Vec<Refund>>,
}

impl FakeLedger {
    /// Tổng đã hoàn của lượt thu `payment`.
    pub(crate) fn refunded(&self, payment: Uuid) -> i64 {
        self.refunds
            .lock()
            .unwrap()
            .iter()
            .filter(|refund| refund.payment_id == payment)
            .map(|r| r.amount.minor())
            .sum()
    }

    pub(crate) fn last_payment(&self) -> Payment {
        self.payments.lock().unwrap().last().cloned().unwrap()
    }
}

#[async_trait::async_trait]
impl Ledger for FakeLedger {
    async fn post(
        &self,
        _: &TenantContext,
        _: Actor,
        _: NewEntry,
    ) -> Result<Submitted<Entry>, Error> {
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
        payment: NewPayment,
    ) -> Result<Submitted<Payment>, Error> {
        let mut payments = self.payments.lock().unwrap();
        if let Some(found) = payments.iter().find(|found| found.reference == payment.reference) {
            return Ok(Submitted::Replayed(found.clone()));
        }
        let now = Timestamp::now();
        let created = Payment {
            id: Uuid::now_v7(),
            org_id: payment.org.as_uuid(),
            book_id: Uuid::now_v7(),
            provider: "fake".to_owned(),
            provider_ref: Some(format!("fake-{}", payments.len() + 1)),
            method: payment.method,
            description: payment.description,
            amount: payment.amount,
            refunded: Money::zero(payment.amount.currency()),
            status: "pending".to_owned(),
            checkout_url: Some("https://pay.example.com/checkout".to_owned()),
            qr_payload: None,
            transfer_code: None,
            bank_account_id: None,
            expires_at: now + SignedDuration::from_mins(15),
            captured_at: None,
            source_module: payment.source_module,
            source_id: payment.source_id,
            reference: payment.reference,
            created_at: now,
            updated_at: now,
        };
        payments.push(created.clone());
        Ok(Submitted::Created(created))
    }
    async fn refund(
        &self,
        _: &TenantContext,
        _: Actor,
        refund: NewRefund,
    ) -> Result<Submitted<Refund>, Error> {
        let mut refunds = self.refunds.lock().unwrap();
        if let Some(found) = refunds.iter().find(|found| found.reference == refund.reference) {
            return Ok(Submitted::Replayed(found.clone()));
        }
        let now = Timestamp::now();
        let created = Refund {
            id: Uuid::now_v7(),
            payment_id: refund.payment,
            amount: refund.amount,
            status: "succeeded".to_owned(),
            provider_ref: None,
            reason: refund.reason,
            reference: refund.reference,
            created_at: now,
            updated_at: now,
        };
        refunds.push(created.clone());
        Ok(Submitted::Created(created))
    }
    async fn payment(&self, _: &TenantContext, id: Uuid) -> Result<Option<Payment>, Error> {
        Ok(self.payments.lock().unwrap().iter().find(|found| found.id == id).cloned())
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

/// Module `ledger` giả: công bố [`FakeLedger`] và khai event `ledger.payment_succeeded`, để test đi đúng
/// đường event thật (outbox → worker → bộ nghe của `bookings`).
pub(crate) struct FakeLedgerModule {
    pub ledger: std::sync::Arc<FakeLedger>,
    pub outbox: std::sync::Arc<Mutex<Option<autoxide_kernel::event::Outbox>>>,
}

impl autoxide_kernel::module::Module for FakeLedgerModule {
    fn name(&self) -> &'static str {
        "ledger"
    }

    fn description(&self) -> &'static str {
        "Fake ledger for the tests of bookings."
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
        context.provide::<dyn Ledger>(self.ledger.clone())
    }
}

impl FakeLedger {
    /// Ghi event "đã thu" của lượt thu `payment` vào outbox của `ledger` giả (chưa giao: cần chạy worker).
    pub(crate) async fn succeed(
        outbox: &Mutex<Option<autoxide_kernel::event::Outbox>>,
        tenant: &TenantContext,
        payment: &Payment,
    ) {
        let outbox = outbox.lock().unwrap().clone().unwrap();
        let payload = serde_json::json!({
            "payment_id": payment.id, "source_module": payment.source_module,
            "source_id": payment.source_id, "reference": payment.reference,
        });
        let transaction = tenant.database().begin_write().await.unwrap();
        outbox
            .record(&transaction, tenant, Actor::system(), None, &PaymentSucceeded(payload))
            .await
            .unwrap();
        transaction.commit().await.unwrap();
    }
}

/// Bộ gửi thông báo giả: nhớ mọi lời gọi `notify` (kể cả trùng khoá) để đếm số lần gửi.
#[derive(Default)]
pub(crate) struct FakeNotifier {
    pub sent: Mutex<Vec<autoxide_notifications::api::Notification>>,
}

#[async_trait::async_trait]
impl autoxide_notifications::api::Notifier for FakeNotifier {
    fn declare(&self, _: autoxide_notifications::api::NotificationKind) -> Result<(), Error> {
        Ok(())
    }

    async fn notify(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        notification: autoxide_notifications::api::Notification,
    ) -> Result<autoxide_notifications::api::Outcome, Error> {
        self.sent.lock().unwrap().push(notification);
        Ok(autoxide_notifications::api::Outcome::default())
    }
}
