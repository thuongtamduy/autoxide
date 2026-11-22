//! Sổ cái giả dùng chung cho test của module khác (feature `testing`, chỉ khai ở `[dev-dependencies]`).
//!
//! Vì sao nằm ở đây mà không ở `autoxide-testkit`: testkit không được biết module nào (luật
//! `dependency` của `cargo xtask check`), mà trait [`Ledger`] là hợp đồng của module `ledger`. Nên
//! bản giả đi cùng hợp đồng, sau một feature để bản build thật không mang nó
//! (D-20261122-module-test-fakes).
//!
//! Hành vi giống `ledger` thật ở những chỗ module khác dựa vào:
//! - bút toán chống trùng theo `reference` (gửi lại trả `Replayed`, không ghi thêm), phải cân Nợ = Có;
//! - tài khoản suy từ khoá, bên thường theo [`OpenAccount`] hay [`PURPOSES`]; số dư theo bên thường;
//! - lượt thu và lượt hoàn chống trùng theo `reference`; hoàn cộng dồn vào `refunded` của lượt thu;
//! - cài đặt dòng tiền mặc định "tự thu";
//! - có [`FakeLedgerModule`] thì phát được event `ledger.payment_succeeded` đúng đường thật
//!   (outbox → dispatcher → bộ nghe của module đang test).
//!
//! Không làm: kỳ kế toán, chuỗi băm, tạm giữ, chi trả, tỉ giá, sổ nhật ký — các hàm đó trả lỗi
//! `internal_error` nói rõ hàm nào chưa có, để test dùng tới thấy ngay.
//!
//! ```ignore
//! let ledger = Arc::new(FakeLedger::new());
//! let app = TestApp::builder(test).module(ledger.module()).module(my_module()).build().await?;
//! // … module đang test tạo lượt thu …
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

/// Tên đối tác của lượt thu khi người gọi không chọn.
pub const FAKE_LEDGER_PROVIDER: &str = "fake";
/// Hạn của lượt thu giả.
const PAYMENT_TTL_MINUTES: i64 = 30;

/// Một dòng bút toán đã ghi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedLine {
    /// ID tài khoản (suy từ khoá: [`FakeLedger::account_id`]).
    pub account: Uuid,
    pub side: Side,
    /// Số tiền, đơn vị nhỏ nhất.
    pub amount: i64,
}

/// Một bút toán đã ghi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedEntry {
    pub id: Uuid,
    pub source_module: String,
    pub reference: String,
    pub lines: Vec<PostedLine>,
}

impl PostedEntry {
    /// Tổng các dòng bên `side`.
    pub fn total(&self, side: Side) -> i64 {
        self.lines.iter().filter(|line| line.side == side).map(|line| line.amount).sum()
    }
}

#[derive(Default)]
struct State {
    entries: Vec<PostedEntry>,
    /// Bút toán đã bị đảo: ID bút toán gốc.
    reversed: Vec<Uuid>,
    /// Bên thường của tài khoản đã mở.
    accounts: HashMap<Uuid, Side>,
    payments: Vec<Payment>,
    /// Đối tác người gọi chọn cho từng lượt thu, cùng thứ tự với `payments`.
    providers: Vec<Option<String>>,
    refunds: Vec<Refund>,
    /// Số lần `post` kế tiếp trả `ledger.period_closed`.
    fail_posts: usize,
    /// Số lần `create_payment` kế tiếp trả `ledger.gateway_not_configured`.
    fail_payments: usize,
    /// `return_url` người gọi gửi cho từng lượt thu qua cổng, theo thứ tự.
    return_urls: Vec<Option<String>>,
}

/// Sổ cái giả, giữ mọi thứ trong bộ nhớ. Công bố vào app bằng
/// `TestApp::builder(…).provide::<dyn Ledger>(ledger.clone())`, hay bằng [`FakeLedger::module`] khi
/// test cần event của `ledger`.
#[derive(Default)]
pub struct FakeLedger {
    state: Mutex<State>,
    outbox: Mutex<Option<Outbox>>,
}

/// Event `ledger.payment_succeeded` (thân JSON như `ledger` thật ghi).
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

    /// Module `ledger` giả bọc sổ này: công bố `dyn Ledger`, khai event `ledger.payment_succeeded`
    /// và bảng outbox, để [`succeed`](Self::succeed) phát được event.
    pub fn module(self: &Arc<Self>) -> FakeLedgerModule {
        FakeLedgerModule { ledger: self.clone() }
    }

    /// ID tài khoản mang khoá `key` (cùng khoá → cùng ID, như `ensure_account`).
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

    /// `count` lần `post` kế tiếp lỗi `ledger.period_closed` (thử đường ghi lại khi sổ từ chối).
    pub fn fail_next_posts(&self, count: usize) {
        lock(&self.state).fail_posts = count;
    }

    /// `count` lần `create_payment` kế tiếp lỗi `ledger.gateway_not_configured` (thử đường đối tác
    /// từ chối khởi tạo lượt thu).
    pub fn fail_next_payments(&self, count: usize) {
        lock(&self.state).fail_payments = count;
    }

    /// `return_url` của từng lần gọi `create_payment` thành công (`None`: không gửi), theo thứ tự.
    pub fn return_urls(&self) -> Vec<Option<String>> {
        lock(&self.state).return_urls.clone()
    }

    /// Bút toán đã ghi, theo thứ tự (lần gửi lại không thêm dòng).
    pub fn entries(&self) -> Vec<PostedEntry> {
        lock(&self.state).entries.clone()
    }

    /// Bút toán mang mã tham chiếu `reference`, nếu có.
    pub fn entry_by_reference(&self, reference: &str) -> Option<PostedEntry> {
        lock(&self.state).entries.iter().find(|entry| entry.reference == reference).cloned()
    }

    /// Mã tham chiếu của các bút toán đã ghi, theo thứ tự.
    pub fn references(&self) -> Vec<String> {
        lock(&self.state).entries.iter().map(|entry| entry.reference.clone()).collect()
    }

    /// Nợ − Có của tài khoản `key`, không xét bên thường.
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

    /// Lượt thu đã tạo, theo thứ tự.
    pub fn payments(&self) -> Vec<Payment> {
        lock(&self.state).payments.clone()
    }

    /// Lượt thu tạo sau cùng.
    pub fn last_payment(&self) -> Option<Payment> {
        lock(&self.state).payments.last().cloned()
    }

    /// Đối tác người gọi chọn cho từng lượt thu (`None`: không chọn), theo thứ tự.
    pub fn providers(&self) -> Vec<Option<String>> {
        lock(&self.state).providers.clone()
    }

    /// Lượt hoàn đã nhận, theo thứ tự.
    pub fn refunds(&self) -> Vec<Refund> {
        lock(&self.state).refunds.clone()
    }

    /// Tổng đã hoàn của lượt thu `payment`, đơn vị nhỏ nhất.
    pub fn refunded(&self, payment: Uuid) -> i64 {
        lock(&self.state)
            .refunds
            .iter()
            .filter(|refund| refund.payment_id == payment)
            .map(|refund| refund.amount.minor())
            .sum()
    }

    /// Đối tác báo đã thu đủ `payment`: đánh dấu đã thu và ghi event `ledger.payment_succeeded` vào
    /// outbox (chưa giao: test chạy dispatcher). Cần app dựng với [`FakeLedger::module`].
    pub async fn succeed(&self, tenant: &TenantContext, payment: &Payment) -> Result<(), Error> {
        self.succeed_with(tenant, payment, payment.amount).await
    }

    /// Như [`succeed`](Self::succeed), với số thực thu `amount` (`ledger` thật cũng báo số thu
    /// thiếu khi đối soát chấp nhận số đã nhận).
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

    /// Ghi event `ledger.payment_succeeded` với thân tự chọn (thử thân thiếu trường, lượt thu không
    /// đi qua sổ giả này…).
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

/// Phần chung của lượt thu qua cổng và lượt thu chuyển khoản.
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

/// Module `ledger` giả: công bố [`FakeLedger`], khai event `ledger.payment_succeeded` và bảng outbox
/// của nó. Đăng ký TRƯỚC module đang test (module đó `require::<dyn Ledger>()`).
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
