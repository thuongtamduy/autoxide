//! A shared fake ledger for other modules' tests (feature `testing`, declared only in `[dev-dependencies]`).
//!
//! Why it lives here and not in `autoxide-testkit`: the testkit must not know any module (the `dependency`
//! rule of `cargo xtask check`), while the [`Ledger`] trait is the contract of the `ledger` module. So the
//! fake ships with the contract, behind a feature so the real build does not carry it
//! (D-20261122-module-test-fakes).
//!
//! Behaves like the real `ledger` where other modules rely on it:
//! - journal entries are deduplicated by `reference` (resending returns `Replayed`, writes nothing more) and must balance Debit = Credit;
//! - accounts derive from keys, the normal side per [`OpenAccount`] or [`PURPOSES`]; balances follow the normal side;
//! - payments and refunds are deduplicated by `reference`; refunds accumulate into the payment's `refunded`;
//! - the cash-flow setting defaults to "self-collect";
//! - with [`FakeLedgerModule`] the event `ledger.payment_succeeded` is emitted along the real path
//!   (outbox → dispatcher → the listener of the module under test).
//!
//! Not implemented: accounting periods, hash chains, holds, payouts, exchange rates, journals — those
//! functions return `internal_error` saying which function is missing, so a test that reaches one sees it at
//! once.
//!
//! ```ignore
//! let ledger = Arc::new(FakeLedger::new());
//! let app = TestApp::builder(test).module(ledger.module()).module(my_module()).build().await?;
//! // … the module under test creates a payment …
//! let payment = ledger.last_payment().unwrap();
//! ledger.succeed(&app.default_tenant(), &payment).await?;
//! ```

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Event, EventInfo, Outbox};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_kernel::time::{Date, Timestamp};
use jiff::SignedDuration;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    AccountKey, CodRemittance, Entry, ExchangeRate, FlowMode, FlowSettings, Hold, JournalLine,
    JournalQuery, Ledger, LineAccount, NewEntry, NewPayment, NewPayout, NewRefund, NewReversal,
    NewTransferPayment, OpenAccount, Payment, Payout, Refund, Settled, Settlement, Side,
    normal_side_of,
};

/// Name of the partner of a payment when the caller does not choose one.
pub const FAKE_LEDGER_PROVIDER: &str = "fake";
/// Deadline of a fake payment.
const PAYMENT_TTL_MINUTES: i64 = 30;

/// A journal entry line that has been recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedLine {
    /// Account ID (derived from the key: [`FakeLedger::account_id`]).
    pub account: Uuid,
    pub side: Side,
    /// Amount, in the smallest unit.
    pub amount: i64,
}

/// A journal entry that has been recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedEntry {
    pub id: Uuid,
    pub source_module: String,
    pub reference: String,
    pub lines: Vec<PostedLine>,
}

impl PostedEntry {
    /// Sum of the lines on side `side`.
    pub fn total(&self, side: Side) -> i64 {
        self.lines.iter().filter(|line| line.side == side).map(|line| line.amount).sum()
    }
}

#[derive(Default)]
struct State {
    entries: Vec<PostedEntry>,
    /// The entry that was reversed: the ID of the original entry.
    reversed: Vec<Uuid>,
    /// The normal side of the opened account.
    accounts: HashMap<Uuid, Side>,
    payments: Vec<Payment>,
    /// The partner the caller chose for each payment, in the same order as `payments`.
    providers: Vec<Option<String>>,
    refunds: Vec<Refund>,
    /// Number of upcoming `post` calls that return `ledger.period_closed`.
    fail_posts: usize,
    /// Number of upcoming `create_payment` calls that return `ledger.gateway_not_configured`.
    fail_payments: usize,
    refuse_refunds: usize,
    /// The `return_url` the caller sent for each payment through the gateway, in order.
    return_urls: Vec<Option<String>>,
}

/// A fake ledger that keeps everything in memory. Publish it into the app with
/// `TestApp::builder(…).provide::<dyn Ledger>(ledger.clone())`, or with [`FakeLedger::module`] when the
/// test needs the events of `ledger`.
#[derive(Default)]
pub struct FakeLedger {
    state: Mutex<State>,
    outbox: Mutex<Option<Outbox>>,
}

/// The event `ledger.payment_succeeded` (JSON body as the real `ledger` records it).
#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
struct PaymentSucceeded(Value);

impl Event for PaymentSucceeded {
    const NAME: &'static str = "ledger.payment_succeeded";
}

// Khoá `Mutex` mà không `unwrap`: test khác panic giữa chừng thì vẫn đọc tiếp dữ liệu bên trong.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn missing<T>(function: &str) -> Result<T, Error> {
    Err(Error::internal(format!("FakeLedger does not implement `{function}`")))
}

impl FakeLedger {
    pub fn new() -> Self {
        FakeLedger::default()
    }

    /// A fake `ledger` module wrapping this ledger: publishes `dyn Ledger`, declares the event
    /// `ledger.payment_succeeded` and the outbox table, so [`succeed`](Self::succeed) can emit the event.
    pub fn module(self: &Arc<Self>) -> FakeLedgerModule {
        FakeLedgerModule { ledger: self.clone() }
    }

    /// Account ID carrying key `key` (same key → same ID, like `ensure_account`).
    pub fn account_id(key: &AccountKey) -> Uuid {
        let party = key
            .counterparty
            .as_ref()
            .map(|party| format!("{}:{}", party.kind, party.id))
            .unwrap_or_default();
        let text = format!("{}|{}|{party}|{}", key.book, key.purpose, key.currency);
        let hash = |salt: &str| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (salt, &text).hash(&mut hasher);
            hasher.finish()
        };
        Uuid::from_u64_pair(hash("a"), hash("b"))
    }

    /// The next `count` `post` calls fail with `ledger.period_closed` (to test the retry path when the ledger refuses).
    pub fn fail_next_posts(&self, count: usize) {
        lock(&self.state).fail_posts = count;
    }

    /// The next `count` `create_payment` calls fail with `ledger.gateway_not_configured` (to test the path
    /// where the partner refuses to start a payment).
    pub fn fail_next_payments(&self, count: usize) {
        lock(&self.state).fail_payments = count;
    }

    /// The `return_url` of each successful `create_payment` call (`None`: not sent), in order.
    pub fn return_urls(&self) -> Vec<Option<String>> {
        lock(&self.state).return_urls.clone()
    }

    /// Entries recorded, in order (a resend adds no lines).
    pub fn entries(&self) -> Vec<PostedEntry> {
        lock(&self.state).entries.clone()
    }

    /// The entry carrying reference code `reference`, if any.
    pub fn entry_by_reference(&self, reference: &str) -> Option<PostedEntry> {
        lock(&self.state).entries.iter().find(|entry| entry.reference == reference).cloned()
    }

    /// Reference codes of the entries recorded, in order.
    pub fn references(&self) -> Vec<String> {
        lock(&self.state).entries.iter().map(|entry| entry.reference.clone()).collect()
    }

    /// Debit − Credit of account `key`, ignoring the normal side.
    pub fn net_debit(&self, key: &AccountKey) -> i64 {
        let id = Self::account_id(key);
        lock(&self.state)
            .entries
            .iter()
            .flat_map(|entry| entry.lines.iter())
            .filter(|line| line.account == id)
            .map(|line| if line.side == Side::Debit { line.amount } else { -line.amount })
            .sum()
    }

    /// Payments created, in order.
    pub fn payments(&self) -> Vec<Payment> {
        lock(&self.state).payments.clone()
    }

    /// The most recently created payment.
    pub fn last_payment(&self) -> Option<Payment> {
        lock(&self.state).payments.last().cloned()
    }

    /// The partner the caller chose for each payment (`None`: none chosen), in order.
    pub fn providers(&self) -> Vec<Option<String>> {
        lock(&self.state).providers.clone()
    }

    /// Refunds received, in order.
    pub fn refunds(&self) -> Vec<Refund> {
        lock(&self.state).refunds.clone()
    }

    /// Total refunded of payment `payment`, in the smallest unit.
    pub fn refunded(&self, payment: Uuid) -> i64 {
        lock(&self.state)
            .refunds
            .iter()
            .filter(|refund| refund.payment_id == payment)
            .map(|refund| refund.amount.minor())
            .sum()
    }

    /// The next `count` `refund` calls are refused with `payment.not_supported` (422: the partner cannot
    /// refund; sending again never helps).
    pub fn refuse_next_refunds(&self, count: usize) {
        lock(&self.state).refuse_refunds = count;
    }

    /// The partner collected `payment`: marks it `succeeded` as the real `ledger` does before it emits
    /// `ledger.payment_succeeded` (for tests that emit the event themselves).
    pub fn collect(&self, payment: Uuid) {
        let mut state = lock(&self.state);
        if let Some(found) = state.payments.iter_mut().find(|found| found.id == payment) {
            found.status = "succeeded".to_owned();
            found.captured_at = Some(Timestamp::now());
            found.updated_at = Timestamp::now();
        }
    }

    /// The partner reports `payment` failed (declined, abandoned): marks it `failed`, as the real `ledger`
    /// does on a failure notification. No event (the source module reads the status when it needs it).
    pub fn fail(&self, payment: Uuid) {
        let mut state = lock(&self.state);
        if let Some(found) = state.payments.iter_mut().find(|found| found.id == payment) {
            found.status = "failed".to_owned();
            found.updated_at = Timestamp::now();
        }
    }

    /// The partner reports `payment` fully collected: marks it collected and records the event
    /// `ledger.payment_succeeded` in the outbox (not delivered: the test runs the dispatcher). Needs an app
    /// built with [`FakeLedger::module`].
    pub async fn succeed(&self, tenant: &TenantContext, payment: &Payment) -> Result<(), Error> {
        self.succeed_with(tenant, payment, payment.amount).await
    }

    /// Like [`succeed`](Self::succeed), with the actual amount received `amount` (the real `ledger` also
    /// reports a short receipt when reconciliation accepts the amount received).
    pub async fn succeed_with(
        &self,
        tenant: &TenantContext,
        payment: &Payment,
        amount: Money,
    ) -> Result<(), Error> {
        {
            let mut state = lock(&self.state);
            if let Some(found) = state.payments.iter_mut().find(|found| found.id == payment.id) {
                found.status = "succeeded".to_owned();
                found.captured_at = Some(Timestamp::now());
                found.updated_at = Timestamp::now();
            }
        }
        self.record_payment_succeeded(
            tenant,
            json!({
                "payment_id": payment.id, "org_id": payment.org_id, "provider": payment.provider,
                "provider_ref": payment.provider_ref, "amount": amount,
                "source_module": payment.source_module, "source_id": payment.source_id,
                "reference": payment.reference,
            }),
        )
        .await
    }

    /// Records the event `ledger.payment_succeeded` with a chosen body (to test a body missing fields, a
    /// payment that does not go through this fake ledger…).
    pub async fn record_payment_succeeded(
        &self,
        tenant: &TenantContext,
        payload: Value,
    ) -> Result<(), Error> {
        // Chép `Outbox` ra rồi mới `await`: không giữ khoá `Mutex` qua điểm `await`.
        let outbox = lock(&self.outbox).clone().ok_or_else(|| {
            Error::internal("FakeLedger has no outbox: build the app with `FakeLedger::module`")
        })?;
        let transaction = tenant.database().begin_write().await?;
        outbox
            .record(&transaction, tenant, Actor::system(), None, &PaymentSucceeded(payload))
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    fn entry_of(entry: &PostedEntry, book: Uuid, new: &NewEntry) -> Entry {
        let now = Timestamp::now();
        Entry {
            id: entry.id,
            book_id: book,
            org_id: book,
            period_id: Uuid::nil(),
            effective_date: new.effective_date.unwrap_or_else(|| now.date_utc()),
            posted_at: now,
            description: new.description.clone(),
            source_module: new.source_module.clone(),
            source_kind: new.source_kind.clone(),
            source_id: new.source_id.clone(),
            reference: new.reference.clone(),
            reverses_entry_id: None,
            reversed_by_entry_id: None,
            actor_user_id: None,
            lines: Vec::new(),
            sequence: None,
            sealed_at: None,
        }
    }

    fn create_payment_of(
        &self,
        org: OrgId,
        request: PaymentRequest,
        transfer: bool,
    ) -> Submitted<Payment> {
        let mut state = lock(&self.state);
        if let Some(found) =
            state.payments.iter().find(|found| found.reference == request.reference)
        {
            return Submitted::Replayed(found.clone());
        }
        let id = Uuid::now_v7();
        let now = Timestamp::now();
        let number = state.payments.len() + 1;
        let payment = Payment {
            id,
            org_id: org.as_uuid(),
            book_id: org.as_uuid(),
            provider: request.provider.clone().unwrap_or_else(|| FAKE_LEDGER_PROVIDER.to_owned()),
            provider_ref: Some(format!("fake-{number}")),
            method: request.method,
            description: request.description,
            amount: request.amount,
            refunded: Money::zero(request.amount.currency()),
            status: "pending".to_owned(),
            checkout_url: (!transfer).then(|| format!("https://pay.example/{id}")),
            qr_payload: transfer.then(|| format!("000201-{id}")),
            transfer_code: transfer.then(|| format!("AX{number:04}")),
            bank_account_id: None,
            expires_at: now + SignedDuration::from_mins(PAYMENT_TTL_MINUTES),
            captured_at: None,
            source_module: request.source_module,
            source_id: request.source_id,
            reference: request.reference,
            created_at: now,
            updated_at: now,
        };
        state.payments.push(payment.clone());
        state.providers.push(request.provider);
        Submitted::Created(payment)
    }
}

/// The part shared by a gateway payment and a bank-transfer payment.
struct PaymentRequest {
    provider: Option<String>,
    method: String,
    amount: Money,
    description: String,
    source_module: String,
    source_id: Option<String>,
    reference: String,
}

#[async_trait::async_trait]
impl Ledger for FakeLedger {
    async fn post(
        &self,
        _: &TenantContext,
        _: Actor,
        entry: NewEntry,
    ) -> Result<Submitted<Entry>, Error> {
        let mut state = lock(&self.state);
        if state.fail_posts > 0 {
            state.fail_posts -= 1;
            return Err(Error::conflict("ledger.period_closed", "The period is closed.")
                .with_param("date", Timestamp::now().date_utc()));
        }
        let book = entry.book.as_uuid();
        if let Some(found) = state.entries.iter().find(|found| {
            found.source_module == entry.source_module && found.reference == entry.reference
        }) {
            return Ok(Submitted::Replayed(Self::entry_of(found, book, &entry)));
        }
        let debit: i64 =
            entry.lines.iter().filter(|l| l.side == Side::Debit).map(|l| l.amount.minor()).sum();
        let credit: i64 =
            entry.lines.iter().filter(|l| l.side == Side::Credit).map(|l| l.amount.minor()).sum();
        if debit != credit || entry.lines.is_empty() {
            return Err(Error::internal(format!(
                "FakeLedger: unbalanced entry `{}` (debit {debit}, credit {credit})",
                entry.reference
            )));
        }
        let mut lines = Vec::with_capacity(entry.lines.len());
        for line in &entry.lines {
            let account = match &line.account {
                LineAccount::Id(id) => *id,
                LineAccount::Key(key) => {
                    let id = Self::account_id(key);
                    let normal = normal_side_of(&key.purpose).unwrap_or(Side::Debit);
                    state.accounts.entry(id).or_insert(normal);
                    id
                }
            };
            lines.push(PostedLine { account, side: line.side, amount: line.amount.minor() });
        }
        let posted = PostedEntry {
            id: Uuid::now_v7(),
            source_module: entry.source_module.clone(),
            reference: entry.reference.clone(),
            lines,
        };
        let made = Self::entry_of(&posted, book, &entry);
        state.entries.push(posted);
        Ok(Submitted::Created(made))
    }

    async fn reverse(
        &self,
        _: &TenantContext,
        _: Actor,
        entry: Uuid,
        reversal: NewReversal,
    ) -> Result<Submitted<Entry>, Error> {
        let mut state = lock(&self.state);
        let Some(original) = state.entries.iter().find(|found| found.id == entry).cloned() else {
            return Err(Error::not_found("ledger.entry_not_found", "The entry does not exist."));
        };
        let new = NewEntry::new(
            OrgId::from_uuid(Uuid::nil()),
            reversal.source_module.clone(),
            "reversal",
            reversal.reference.clone(),
            "Reversal",
            Vec::new(),
        );
        if let Some(found) = state.entries.iter().find(|found| {
            found.source_module == reversal.source_module && found.reference == reversal.reference
        }) {
            return Ok(Submitted::Replayed(Self::entry_of(found, Uuid::nil(), &new)));
        }
        if state.reversed.contains(&entry) {
            return Err(Error::conflict(
                "ledger.already_reversed",
                "The entry has already been reversed.",
            ));
        }
        let flip = |side: Side| if side == Side::Debit { Side::Credit } else { Side::Debit };
        let posted = PostedEntry {
            id: Uuid::now_v7(),
            source_module: reversal.source_module,
            reference: reversal.reference,
            lines: original
                .lines
                .iter()
                .map(|line| PostedLine { side: flip(line.side), ..line.clone() })
                .collect(),
        };
        let mut made = Self::entry_of(&posted, Uuid::nil(), &new);
        made.reverses_entry_id = Some(entry);
        state.reversed.push(entry);
        state.entries.push(posted);
        Ok(Submitted::Created(made))
    }

    async fn ensure_account(&self, _: &TenantContext, key: AccountKey) -> Result<Uuid, Error> {
        let id = Self::account_id(&key);
        let normal = normal_side_of(&key.purpose).unwrap_or(Side::Debit);
        lock(&self.state).accounts.entry(id).or_insert(normal);
        Ok(id)
    }

    async fn open_account(&self, _: &TenantContext, account: OpenAccount) -> Result<Uuid, Error> {
        let id = Self::account_id(&account.key);
        let normal = account
            .normal_side
            .or_else(|| normal_side_of(&account.key.purpose))
            .unwrap_or(Side::Debit);
        lock(&self.state).accounts.entry(id).or_insert(normal);
        Ok(id)
    }

    async fn balance(
        &self,
        tenant: &TenantContext,
        key: AccountKey,
        as_of: Option<Date>,
    ) -> Result<Money, Error> {
        let mut balances = self.balances(tenant, std::slice::from_ref(&key), as_of).await?;
        Ok(balances.pop().unwrap_or_else(|| Money::zero(key.currency)))
    }

    async fn entry(&self, _: &TenantContext, _: Uuid) -> Result<Option<Entry>, Error> {
        missing("entry")
    }

    async fn create_payment(
        &self,
        _: &TenantContext,
        _: Actor,
        payment: NewPayment,
    ) -> Result<Submitted<Payment>, Error> {
        {
            let mut state = lock(&self.state);
            if state.fail_payments > 0 {
                state.fail_payments -= 1;
                return Err(Error::new(
                    autoxide_kernel::error::ErrorKind::Validation,
                    "ledger.gateway_not_configured",
                    "The payment partner `{provider}` is not set up on this server.",
                )
                .with_param(
                    "provider",
                    payment.provider.as_deref().unwrap_or(FAKE_LEDGER_PROVIDER),
                ));
            }
            state.return_urls.push(payment.return_url.clone());
        }
        let org = payment.org;
        let request = PaymentRequest {
            provider: payment.provider,
            method: payment.method,
            amount: payment.amount,
            description: payment.description,
            source_module: payment.source_module,
            source_id: payment.source_id,
            reference: payment.reference,
        };
        Ok(self.create_payment_of(org, request, false))
    }

    async fn refund(
        &self,
        _: &TenantContext,
        _: Actor,
        refund: NewRefund,
    ) -> Result<Submitted<Refund>, Error> {
        let mut state = lock(&self.state);
        if let Some(found) = state
            .refunds
            .iter()
            .find(|found| found.payment_id == refund.payment && found.reference == refund.reference)
        {
            return Ok(Submitted::Replayed(found.clone()));
        }
        if state.refuse_refunds > 0 {
            state.refuse_refunds -= 1;
            return Err(autoxide_kernel::payment::not_supported(FAKE_LEDGER_PROVIDER));
        }
        let now = Timestamp::now();
        if let Some(payment) = state.payments.iter_mut().find(|found| found.id == refund.payment) {
            payment.refunded = Money::new(
                payment.refunded.minor() + refund.amount.minor(),
                payment.amount.currency(),
            );
            payment.updated_at = now;
        }
        let made = Refund {
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
        state.refunds.push(made.clone());
        Ok(Submitted::Created(made))
    }

    async fn payment(&self, _: &TenantContext, id: Uuid) -> Result<Option<Payment>, Error> {
        Ok(lock(&self.state).payments.iter().find(|found| found.id == id).cloned())
    }

    async fn balances(
        &self,
        _: &TenantContext,
        keys: &[AccountKey],
        _: Option<Date>,
    ) -> Result<Vec<Money>, Error> {
        let normals: Vec<Side> = {
            let state = lock(&self.state);
            keys.iter()
                .map(|key| {
                    state
                        .accounts
                        .get(&Self::account_id(key))
                        .copied()
                        .or_else(|| normal_side_of(&key.purpose))
                        .unwrap_or(Side::Debit)
                })
                .collect()
        };
        Ok(keys
            .iter()
            .zip(normals)
            .map(|(key, normal)| {
                let net = self.net_debit(key);
                Money::new(if normal == Side::Debit { net } else { -net }, key.currency)
            })
            .collect())
    }

    async fn flow_settings(&self, _: &TenantContext, org: OrgId) -> Result<FlowSettings, Error> {
        Ok(FlowSettings {
            org_id: org.as_uuid(),
            source_org_id: None,
            mode: FlowMode::SelfCollect,
            payment_provider: Some(FAKE_LEDGER_PROVIDER.to_owned()),
            payout_provider: None,
            hold_days: 0,
            cod_remittance: CodRemittance::PartnerCollection,
            inherited: true,
            updated_at: None,
        })
    }

    async fn settle(
        &self,
        _: &TenantContext,
        _: Actor,
        _: Settlement,
    ) -> Result<Submitted<Settled>, Error> {
        missing("settle")
    }

    async fn hold_status(
        &self,
        _: &TenantContext,
        _: &str,
        _: &str,
    ) -> Result<Option<Hold>, Error> {
        missing("hold_status")
    }

    async fn create_payout(
        &self,
        _: &TenantContext,
        _: Actor,
        _: NewPayout,
    ) -> Result<Submitted<Payout>, Error> {
        missing("create_payout")
    }

    async fn create_transfer_payment(
        &self,
        _: &TenantContext,
        _: Actor,
        payment: NewTransferPayment,
    ) -> Result<Submitted<Payment>, Error> {
        let org = payment.org;
        let request = PaymentRequest {
            provider: None,
            method: "bank_transfer".to_owned(),
            amount: payment.amount,
            description: payment.description,
            source_module: payment.source_module,
            source_id: payment.source_id,
            reference: payment.reference,
        };
        Ok(self.create_payment_of(org, request, true))
    }

    async fn exchange_rate(
        &self,
        _: &TenantContext,
        _: Currency,
        _: Currency,
        _: Date,
    ) -> Result<Option<ExchangeRate>, Error> {
        Ok(None)
    }

    async fn book_currency(&self, _: &TenantContext, _: OrgId) -> Result<Currency, Error> {
        Ok(Currency::VND)
    }

    async fn journal(
        &self,
        _: &TenantContext,
        _: JournalQuery,
    ) -> Result<Page<JournalLine>, Error> {
        missing("journal")
    }
}

/// A fake `ledger` module: publishes [`FakeLedger`], declares the event `ledger.payment_succeeded` and its
/// outbox table. Register BEFORE the module under test (that module calls `require::<dyn Ledger>()`).
pub struct FakeLedgerModule {
    ledger: Arc<FakeLedger>,
}

impl Module for FakeLedgerModule {
    fn name(&self) -> &'static str {
        "ledger"
    }

    fn description(&self) -> &'static str {
        "In-memory ledger for tests (feature `testing` of autoxide-ledger)."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
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

    fn events(&self) -> Vec<EventInfo> {
        vec![EventInfo::new("ledger.payment_succeeded", "A payment was collected.")]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *lock(&self.ledger.outbox) = Some(context.outbox()?);
        context.provide::<dyn Ledger>(self.ledger.clone())
    }
}
