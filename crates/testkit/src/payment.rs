//! A fake payment gateway for tests (like `FakeTextModel`): keeps everything in memory, signs webhooks with
//! a fixed key, can simulate an unresponsive partner and build statements.

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{
    BankAccountRef, BankBatch, BankDirection, BankFeed, BankTransaction, GatewayCapabilities,
    GatewayNotification, GatewayPayment, GatewayPaymentStatus, GatewayPayout, GatewayRefund,
    NotificationKind, PaymentGateway, PaymentIntent, PayoutGateway, PayoutIntent, PayoutStatus,
    RefundIntent, RefundStatus, StatementLine, StatementPage, provider_unavailable,
    signature_invalid,
};
use autoxide_kernel::tenancy::TenantId;
use axum::http::HeaderMap;
use serde_json::{Value, json};
use uuid::Uuid;

/// Name of the fake gateway (webhook path `/api/v1/ledger/webhooks/fake`).
pub const FAKE_PROVIDER: &str = "fake";
const SECRET: &str = "fake-gateway-secret";
const SIGNATURE: &str = "x-fake-signature";
const TIMESTAMP: &str = "x-fake-timestamp";
/// Replay-protection window of the signature, in seconds.
const WINDOW: i64 = 300;

/// A fake payment gateway. Contribute it to the app with
/// `TestApp::builder(…).contribute::<dyn PaymentGateway>(gateway.clone())`.
#[derive(Default)]
pub struct FakePaymentGateway {
    /// Collections created, by payment ID: (code at the partner, amount).
    pub payments: Mutex<HashMap<Uuid, (String, Money)>>,
    /// Refunds received, in order.
    pub refunds: Mutex<Vec<RefundIntent>>,
    /// Number of upcoming `create_payment` / `refund` calls that return `payment.provider_unavailable`.
    unavailable: AtomicUsize,
    /// A refund returns `Pending` (the partner reports the result later via webhook) instead of finishing at once.
    refunds_pending: AtomicBool,
    statement: Mutex<Vec<StatementLine>>,
    /// No `split` and `collection_on_behalf` (to try a cash-flow mode the gateway does not support).
    limited: AtomicBool,
}

impl FakePaymentGateway {
    pub fn new() -> Self {
        FakePaymentGateway::default()
    }

    /// The next `count` calls to the partner fail as if the partner did not respond.
    pub fn fail_next(&self, count: usize) {
        self.unavailable.store(count, Ordering::SeqCst);
    }

    /// Refunds wait for a webhook instead of finishing at once.
    pub fn refunds_pending(&self, pending: bool) {
        self.refunds_pending.store(pending, Ordering::SeqCst);
    }

    /// Removes the gateway's ability to split money and collect on behalf (both are on by default).
    pub fn limited(&self, limited: bool) {
        self.limited.store(limited, Ordering::SeqCst);
    }

    /// Sets the statement the partner returns.
    pub fn set_statement(&self, lines: Vec<StatementLine>) {
        if let Ok(mut slot) = self.statement.lock() {
            *slot = lines;
        }
    }

    /// Transaction code at the partner for payment `payment_id` (after `create_payment`).
    pub fn provider_ref(&self, payment_id: Uuid) -> Option<String> {
        self.payments.lock().ok()?.get(&payment_id).map(|(reference, _)| reference.clone())
    }

    fn take_failure(&self) -> Result<(), Error> {
        let failed = self
            .unavailable
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
            .is_ok();
        if failed { Err(provider_unavailable()) } else { Ok(()) }
    }

    fn sign(timestamp: i64, body: &[u8]) -> String {
        autoxide_kernel::secret::sha256(&format!(
            "{SECRET}|{timestamp}|{}",
            String::from_utf8_lossy(body)
        ))
    }

    /// A signed webhook (header, body) carrying the notification `body` (JSON: `event_id`, `kind`
    /// (`payment_succeeded` / `payment_failed` / `refund_succeeded` / `refund_failed`),
    /// `provider_ref`, `payment_id`, `amount` (smallest unit), `currency`, `reason`,
    /// `refund_provider_ref`).
    pub fn webhook(&self, body: &Value) -> (Vec<(String, String)>, Vec<u8>) {
        Self::signed(body, Timestamp::now().as_unix_seconds())
    }

    /// Like [`webhook`](Self::webhook) with a chosen timestamp (to test replay protection).
    pub fn signed(body: &Value, timestamp: i64) -> (Vec<(String, String)>, Vec<u8>) {
        let bytes = body.to_string().into_bytes();
        let headers = vec![
            (SIGNATURE.to_owned(), Self::sign(timestamp, &bytes)),
            (TIMESTAMP.to_owned(), timestamp.to_string()),
        ];
        (headers, bytes)
    }

    /// A "collected `amount`" webhook for payment `payment_id`, notification ID `event_id`.
    pub fn paid(
        &self,
        payment_id: Uuid,
        amount: Money,
        event_id: &str,
    ) -> (Vec<(String, String)>, Vec<u8>) {
        let provider_ref = self.provider_ref(payment_id).unwrap_or_default();
        self.webhook(&json!({
            "event_id": event_id, "kind": "payment_succeeded", "provider_ref": provider_ref,
            "payment_id": payment_id, "amount": amount.minor(), "currency": amount.currency().code(),
        }))
    }
}

fn text<'a>(body: &'a Value, key: &str) -> Result<&'a str, Error> {
    body[key].as_str().ok_or_else(signature_invalid)
}

#[async_trait::async_trait]
impl PaymentGateway for FakePaymentGateway {
    fn provider(&self) -> &'static str {
        FAKE_PROVIDER
    }

    fn capabilities(&self) -> GatewayCapabilities {
        let basic = GatewayCapabilities::new(&["card", "wallet", "vietqr"])
            .with_refunds(true)
            .with_statements();
        if self.limited.load(Ordering::SeqCst) {
            basic
        } else {
            basic.with_split().with_collection_on_behalf()
        }
    }

    async fn create_payment(
        &self,
        _tenant: TenantId,
        intent: &PaymentIntent,
    ) -> Result<GatewayPayment, Error> {
        self.take_failure()?;
        let reference = format!("fake_{}", intent.payment_id.simple());
        if let Ok(mut payments) = self.payments.lock() {
            payments.entry(intent.payment_id).or_insert((reference.clone(), intent.amount));
        }
        Ok(GatewayPayment::new(reference.clone())
            .checkout_url(format!("https://pay.example.test/{reference}")))
    }

    async fn refund(
        &self,
        _tenant: TenantId,
        intent: &RefundIntent,
    ) -> Result<GatewayRefund, Error> {
        self.take_failure()?;
        if let Ok(mut refunds) = self.refunds.lock() {
            refunds.push(intent.clone());
        }
        let status = if self.refunds_pending.load(Ordering::SeqCst) {
            RefundStatus::Pending
        } else {
            RefundStatus::Succeeded
        };
        Ok(GatewayRefund::new(format!("fake_refund_{}", intent.refund_id.simple()), status))
    }

    async fn payment_status(
        &self,
        _tenant: TenantId,
        provider_ref: &str,
    ) -> Result<GatewayPaymentStatus, Error> {
        let known = self
            .payments
            .lock()
            .ok()
            .is_some_and(|payments| payments.values().any(|(found, _)| found == provider_ref));
        Ok(if known { GatewayPaymentStatus::Pending } else { GatewayPaymentStatus::Expired })
    }

    async fn verify_webhook(
        &self,
        _tenant: TenantId,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<GatewayNotification, Error> {
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let (Some(signature), Some(timestamp)) = (header(SIGNATURE), header(TIMESTAMP)) else {
            return Err(signature_invalid());
        };
        let timestamp: i64 = timestamp.parse().map_err(|_| signature_invalid())?;
        if (Timestamp::now().as_unix_seconds() - timestamp).abs() > WINDOW {
            return Err(signature_invalid());
        }
        if Self::sign(timestamp, body) != signature {
            return Err(signature_invalid());
        }
        let body: Value = serde_json::from_slice(body).map_err(|_| signature_invalid())?;
        let currency = Currency::parse(text(&body, "currency")?)?;
        let amount = Money::new(body["amount"].as_i64().ok_or_else(signature_invalid)?, currency);
        let refund_ref = || text(&body, "refund_provider_ref").map(str::to_owned);
        let kind = match text(&body, "kind")? {
            "payment_succeeded" => NotificationKind::PaymentSucceeded,
            "payment_failed" => NotificationKind::PaymentFailed {
                reason: body["reason"].as_str().unwrap_or("declined").to_owned(),
            },
            "refund_succeeded" => {
                NotificationKind::RefundSucceeded { refund_provider_ref: refund_ref()? }
            }
            "refund_failed" => {
                NotificationKind::RefundFailed { refund_provider_ref: refund_ref()? }
            }
            _ => return Err(signature_invalid()),
        };
        let at = Timestamp::from_unix_seconds(timestamp).unwrap_or_else(Timestamp::now);
        let mut notification = GatewayNotification::new(
            text(&body, "event_id")?,
            kind,
            text(&body, "provider_ref")?,
            amount,
            at,
        );
        if let Some(id) = body["payment_id"].as_str().and_then(|id| id.parse().ok()) {
            notification = notification.payment_id(id);
        }
        Ok(notification)
    }

    async fn statement(
        &self,
        _tenant: TenantId,
        from: Date,
        to: Date,
        _cursor: Option<String>,
    ) -> Result<StatementPage, Error> {
        let lines = self
            .statement
            .lock()
            .map(|lines| {
                lines
                    .iter()
                    .filter(|line| line.booked_on >= from && line.booked_on <= to)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Ok(StatementPage::new(lines, None))
    }
}

// ---- Chi trả giả --------------------------------------------------------------------------------

/// Result [`FakePayoutGateway`] returns for the next payout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FakePayoutOutcome {
    #[default]
    Succeed,
    Fail,
    /// The partner accepts the order and reports the result later (`payout_status` returns per [`FakePayoutGateway::settle`]).
    Pending,
}

/// A fake payout partner. Contribute it to the app with
/// `TestApp::builder(…).contribute::<dyn PayoutGateway>(payouts.clone())`.
#[derive(Default)]
pub struct FakePayoutGateway {
    /// Payout orders received (once per `payout_id`), in order.
    pub payouts: Mutex<Vec<PayoutIntent>>,
    outcome: Mutex<FakePayoutOutcome>,
    /// Final result of pending orders, by code at the partner.
    finals: Mutex<HashMap<String, PayoutStatus>>,
    unavailable: AtomicUsize,
}

impl FakePayoutGateway {
    pub fn new() -> Self {
        FakePayoutGateway::default()
    }

    /// Results of the upcoming payouts.
    pub fn outcome(&self, outcome: FakePayoutOutcome) {
        if let Ok(mut slot) = self.outcome.lock() {
            *slot = outcome;
        }
    }

    /// The next `count` `payout` calls fail as if the partner did not respond.
    pub fn fail_next(&self, count: usize) {
        self.unavailable.store(count, Ordering::SeqCst);
    }

    /// Finishes the pending order of payout `payout_id`: success or failure.
    pub fn settle(&self, payout_id: Uuid, succeeded: bool) {
        let status = if succeeded {
            PayoutStatus::Succeeded
        } else {
            PayoutStatus::Failed { reason: "rejected by the bank".to_owned() }
        };
        if let Ok(mut finals) = self.finals.lock() {
            finals.insert(Self::reference(payout_id), status);
        }
    }

    /// Number of payout orders received.
    pub fn count(&self) -> usize {
        self.payouts.lock().map(|payouts| payouts.len()).unwrap_or(0)
    }

    fn reference(payout_id: Uuid) -> String {
        format!("fake_payout_{}", payout_id.simple())
    }
}

#[async_trait::async_trait]
impl PayoutGateway for FakePayoutGateway {
    fn provider(&self) -> &'static str {
        FAKE_PROVIDER
    }

    async fn payout(
        &self,
        _tenant: TenantId,
        intent: &PayoutIntent,
    ) -> Result<GatewayPayout, Error> {
        let failed = self
            .unavailable
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
            .is_ok();
        if failed {
            return Err(provider_unavailable());
        }
        if let Ok(mut payouts) = self.payouts.lock()
            && !payouts.iter().any(|seen| seen.payout_id == intent.payout_id)
        {
            payouts.push(intent.clone());
        }
        let outcome = self.outcome.lock().map(|slot| *slot).unwrap_or_default();
        let status = match outcome {
            FakePayoutOutcome::Succeed => PayoutStatus::Succeeded,
            FakePayoutOutcome::Fail => {
                PayoutStatus::Failed { reason: "rejected by the bank".to_owned() }
            }
            FakePayoutOutcome::Pending => PayoutStatus::Pending,
        };
        Ok(GatewayPayout::new(Self::reference(intent.payout_id), status))
    }

    async fn payout_status(
        &self,
        _tenant: TenantId,
        provider_ref: &str,
    ) -> Result<PayoutStatus, Error> {
        Ok(self
            .finals
            .lock()
            .ok()
            .and_then(|finals| finals.get(provider_ref).cloned())
            .unwrap_or(PayoutStatus::Pending))
    }
}

// ---- Biến động số dư giả -------------------------------------------------------------------------

const BANK_SECRET: &str = "fake-bank-feed-secret";
const BANK_SIGNATURE: &str = "x-fake-bank-signature";

/// A fake source of balance movements: the pull kind returns transactions [`push`](Self::push)ed per account
/// (the cursor code is the number of transactions already read); the push kind signs the body with a fixed
/// key. Contribute it to the app with `TestApp::builder(…).contribute::<dyn BankFeed>(feed.clone())`.
#[derive(Default)]
pub struct FakeBankFeed {
    transactions: Mutex<Vec<BankTransaction>>,
    /// Number of `fetch` calls.
    pub fetches: AtomicUsize,
}

impl FakeBankFeed {
    pub fn new() -> Self {
        FakeBankFeed::default()
    }

    /// Adds a movement (for the pull kind).
    pub fn push(&self, transaction: BankTransaction) {
        if let Ok(mut transactions) = self.transactions.lock() {
            transactions.push(transaction);
        }
    }

    /// A deposit into account `account_number`.
    pub fn credit(
        txn_id: &str,
        account_number: &str,
        amount: Money,
        memo: &str,
    ) -> BankTransaction {
        BankTransaction::new(
            txn_id,
            account_number,
            BankDirection::Credit,
            amount,
            memo,
            Timestamp::now(),
        )
    }

    /// A signed webhook (header, body) carrying the movements `transactions`.
    pub fn webhook(transactions: &[BankTransaction]) -> (Vec<(String, String)>, Vec<u8>) {
        let items: Vec<Value> = transactions
            .iter()
            .map(|txn| {
                json!({
                    "id": txn.provider_txn_id, "account": txn.account_number,
                    "direction": txn.direction.as_str(), "amount": txn.amount.minor(),
                    "currency": txn.amount.currency().code(), "memo": txn.memo,
                    "booked_at": txn.booked_at.as_unix_seconds(),
                })
            })
            .collect();
        let bytes = json!({"transactions": items}).to_string().into_bytes();
        let signature = Self::sign(&bytes);
        (vec![(BANK_SIGNATURE.to_owned(), signature)], bytes)
    }

    fn sign(body: &[u8]) -> String {
        autoxide_kernel::secret::sha256(&format!("{BANK_SECRET}|{}", String::from_utf8_lossy(body)))
    }
}

#[async_trait::async_trait]
impl BankFeed for FakeBankFeed {
    fn provider(&self) -> &'static str {
        FAKE_PROVIDER
    }

    async fn verify_webhook(
        &self,
        _tenant: TenantId,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Vec<BankTransaction>, Error> {
        let signature = headers.get(BANK_SIGNATURE).and_then(|value| value.to_str().ok());
        if signature != Some(Self::sign(body).as_str()) {
            return Err(signature_invalid());
        }
        let body: Value = serde_json::from_slice(body).map_err(|_| signature_invalid())?;
        let items = body["transactions"].as_array().ok_or_else(signature_invalid)?;
        let mut found = Vec::with_capacity(items.len());
        for item in items {
            let currency = Currency::parse(text(item, "currency")?)?;
            let amount =
                Money::new(item["amount"].as_i64().ok_or_else(signature_invalid)?, currency);
            let direction = if text(item, "direction")? == "credit" {
                BankDirection::Credit
            } else {
                BankDirection::Debit
            };
            let at = item["booked_at"]
                .as_i64()
                .and_then(Timestamp::from_unix_seconds)
                .unwrap_or_else(Timestamp::now);
            found.push(BankTransaction::new(
                text(item, "id")?,
                text(item, "account")?,
                direction,
                amount,
                text(item, "memo")?,
                at,
            ));
        }
        Ok(found)
    }

    async fn fetch(
        &self,
        _tenant: TenantId,
        account: &BankAccountRef,
        cursor: Option<String>,
    ) -> Result<BankBatch, Error> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        let read: usize = cursor.and_then(|cursor| cursor.parse().ok()).unwrap_or(0);
        let mine: Vec<BankTransaction> = self
            .transactions
            .lock()
            .map(|transactions| {
                transactions
                    .iter()
                    .filter(|txn| txn.account_number == account.account_number)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let fresh: Vec<BankTransaction> = mine.iter().skip(read).cloned().collect();
        Ok(BankBatch::new(fresh, Some(mine.len().to_string())))
    }
}

/// Name of the fake exchange-rate source (`provider:fake` in the source column).
pub const FAKE_RATE_PROVIDER: &str = "fake";

/// A fake exchange-rate source for tests: returns the loaded rates for exactly `base` (the date of the call),
/// or the error `payment.provider_unavailable` when [`FakeExchangeRateSource::fail`].
#[derive(Debug, Default)]
pub struct FakeExchangeRateSource {
    rates: Mutex<Vec<(Currency, Currency, rust_decimal::Decimal)>>,
    failing: AtomicBool,
    /// Number of `rates` calls.
    pub calls: AtomicUsize,
}

impl FakeExchangeRateSource {
    pub fn new() -> Self {
        FakeExchangeRateSource::default()
    }

    /// Loads the rate 1 `base` = `rate` `quote` (decimal string, e.g. `"25345.5"`).
    pub fn set(&self, base: Currency, quote: Currency, rate: &str) {
        let Ok(rate) = autoxide_kernel::decimal::parse(rate) else { return };
        if let Ok(mut rates) = self.rates.lock() {
            rates.retain(|(b, q, _)| !(*b == base && *q == quote));
            rates.push((base, quote, rate));
        }
    }

    /// `true`: every later call returns the partner-did-not-respond error.
    pub fn fail(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl autoxide_kernel::payment::ExchangeRateSource for FakeExchangeRateSource {
    fn provider(&self) -> &'static str {
        FAKE_RATE_PROVIDER
    }

    async fn rates(
        &self,
        _tenant: TenantId,
        base: Currency,
        on: Date,
    ) -> Result<Vec<autoxide_kernel::payment::QuotedRate>, Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failing.load(Ordering::SeqCst) {
            return Err(provider_unavailable());
        }
        let rates = self.rates.lock().map(|rates| rates.clone()).unwrap_or_default();
        Ok(rates
            .into_iter()
            .filter(|(b, _, _)| *b == base)
            .map(|(b, q, rate)| autoxide_kernel::payment::QuotedRate::new(b, q, rate, on))
            .collect())
    }
}
