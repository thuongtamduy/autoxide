//! Payments through a licensed partner (step 36, D-187): a shared trait so the `ledger` module can call a
//! payment gateway without depending on any one partner. **The core never holds money itself**: money flows
//! from the buyer to the partner (VNPay, MoMo, banks…); the ledger only records obligations and reconciles
//! against statements.
//!
//! A partner's connector is a separate module that depends only on the kernel, registers BEFORE `ledger`, and
//! contributes through `context.contribute::<dyn PaymentGateway>(…)` — several partners can run at once.
//! Every function takes the tenant (`TenantId`): the connector reads that tenant's connection details itself
//! (per-tenant secret store) and never reads environment variables for anything that belongs to a tenant.
//! Tests use the testkit's `FakePaymentGateway`.
use crate::time::{Date, Timestamp};
use uuid::Uuid;

use crate::error::{Error, ErrorKind};
use crate::money::{Currency, Money};
use crate::tenancy::TenantId;

/// What a gateway can do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct GatewayCapabilities {
    /// Accepted methods (`card`, `wallet`, `bank_transfer`, `vietqr`…).
    pub methods: Vec<String>,
    pub refunds: bool,
    pub partial_refunds: bool,
    /// Splits money at the gateway among several recipients.
    pub split: bool,
    /// The partner collects and disburses on behalf of the marketplace (licensed).
    pub collection_on_behalf: bool,
    /// Provides statements for reconciliation.
    pub statements: bool,
}

impl GatewayCapabilities {
    pub fn new(methods: &[&str]) -> Self {
        GatewayCapabilities {
            methods: methods.iter().map(|method| (*method).to_owned()).collect(),
            ..GatewayCapabilities::default()
        }
    }

    #[must_use]
    pub fn with_refunds(mut self, partial: bool) -> Self {
        self.refunds = true;
        self.partial_refunds = partial;
        self
    }

    #[must_use]
    pub fn with_split(mut self) -> Self {
        self.split = true;
        self
    }

    #[must_use]
    pub fn with_collection_on_behalf(mut self) -> Self {
        self.collection_on_behalf = true;
        self
    }

    #[must_use]
    pub fn with_statements(mut self) -> Self {
        self.statements = true;
        self
    }
}

/// The share of money paid to one recipient at the gateway (only when the gateway has `split`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    /// Recipient code at the partner.
    pub payee: String,
    pub amount: Money,
}

/// A collection to create at the partner.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PaymentIntent {
    /// Payment ID in the ledger, sent as the merchant's order code: the partner deduplicates on this code, so
    /// calling again with the same ID does not create two transactions.
    pub payment_id: Uuid,
    pub amount: Money,
    pub method: String,
    /// Short text shown to the buyer; contains no personal data.
    pub description: String,
    /// Page the buyer returns to after paying.
    pub return_url: Option<String>,
    pub expires_at: Timestamp,
    pub splits: Vec<Split>,
}

impl PaymentIntent {
    pub fn new(
        payment_id: Uuid,
        amount: Money,
        method: impl Into<String>,
        description: impl Into<String>,
        expires_at: Timestamp,
    ) -> Self {
        PaymentIntent {
            payment_id,
            amount,
            method: method.into(),
            description: description.into(),
            return_url: None,
            expires_at,
            splits: Vec::new(),
        }
    }

    #[must_use]
    pub fn return_url(mut self, url: Option<String>) -> Self {
        self.return_url = url;
        self
    }
}

/// A collection the partner has just created.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GatewayPayment {
    /// Transaction code at the partner.
    pub provider_ref: String,
    /// The partner's payment page, if any.
    pub checkout_url: Option<String>,
    /// QR string, if the partner returns one.
    pub qr_payload: Option<String>,
}

impl GatewayPayment {
    pub fn new(provider_ref: impl Into<String>) -> Self {
        GatewayPayment { provider_ref: provider_ref.into(), checkout_url: None, qr_payload: None }
    }

    #[must_use]
    pub fn checkout_url(mut self, url: impl Into<String>) -> Self {
        self.checkout_url = Some(url.into());
        self
    }
}

/// A refund to send to the partner.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RefundIntent {
    /// Refund ID in the ledger (the partner deduplicates on this code).
    pub refund_id: Uuid,
    /// Transaction code of the collection at the partner.
    pub payment_provider_ref: String,
    pub amount: Money,
    pub reason: String,
}

impl RefundIntent {
    pub fn new(
        refund_id: Uuid,
        payment_provider_ref: impl Into<String>,
        amount: Money,
        reason: impl Into<String>,
    ) -> Self {
        RefundIntent {
            refund_id,
            payment_provider_ref: payment_provider_ref.into(),
            amount,
            reason: reason.into(),
        }
    }
}

/// Status of a refund at the partner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefundStatus {
    Pending,
    Succeeded,
    Failed,
}

/// Result of sending a refund.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GatewayRefund {
    pub provider_ref: String,
    pub status: RefundStatus,
}

impl GatewayRefund {
    pub fn new(provider_ref: impl Into<String>, status: RefundStatus) -> Self {
        GatewayRefund { provider_ref: provider_ref.into(), status }
    }
}

/// Status of a collection at the partner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayPaymentStatus {
    Pending,
    /// `amount` collected (may differ from the amount requested).
    Succeeded {
        amount: Money,
    },
    Failed {
        reason: String,
    },
    Expired,
}

/// Kind of notification the partner sends through a webhook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationKind {
    PaymentSucceeded,
    PaymentFailed {
        reason: String,
    },
    /// A refund (code at the partner) has completed.
    RefundSucceeded {
        refund_provider_ref: String,
    },
    RefundFailed {
        refund_provider_ref: String,
    },
}

/// A notification whose signature has been verified.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GatewayNotification {
    /// Notification code at the partner: if redelivered several times with the same code, it is processed once.
    pub event_id: String,
    pub kind: NotificationKind,
    /// Transaction code of the collection at the partner.
    pub provider_ref: String,
    /// Payment ID in the ledger (the merchant's order code), if the partner sent it along.
    pub payment_id: Option<Uuid>,
    pub amount: Money,
    pub occurred_at: Timestamp,
}

impl GatewayNotification {
    pub fn new(
        event_id: impl Into<String>,
        kind: NotificationKind,
        provider_ref: impl Into<String>,
        amount: Money,
        occurred_at: Timestamp,
    ) -> Self {
        GatewayNotification {
            event_id: event_id.into(),
            kind,
            provider_ref: provider_ref.into(),
            payment_id: None,
            amount,
            occurred_at,
        }
    }

    #[must_use]
    pub fn payment_id(mut self, id: Uuid) -> Self {
        self.payment_id = Some(id);
        self
    }
}

/// Kind of a partner's statement line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementKind {
    Payment,
    Refund,
    Fee,
    Payout,
}

impl StatementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StatementKind::Payment => "payment",
            StatementKind::Refund => "refund",
            StatementKind::Fee => "fee",
            StatementKind::Payout => "payout",
        }
    }
}

/// One statement line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatementLine {
    /// Line code at the partner (unique).
    pub line_id: String,
    /// Related transaction code (collection, refund…).
    pub provider_ref: String,
    pub kind: StatementKind,
    /// Transaction amount (always positive).
    pub amount: Money,
    /// Fee the partner charges on this line (0 if none).
    pub fee: Money,
    pub booked_on: Date,
}

impl StatementLine {
    pub fn new(
        line_id: impl Into<String>,
        provider_ref: impl Into<String>,
        kind: StatementKind,
        amount: Money,
        booked_on: Date,
    ) -> Self {
        StatementLine {
            line_id: line_id.into(),
            provider_ref: provider_ref.into(),
            kind,
            fee: Money::zero(amount.currency()),
            amount,
            booked_on,
        }
    }

    #[must_use]
    pub fn fee(mut self, fee: Money) -> Self {
        self.fee = fee;
        self
    }
}

/// One page of a statement.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatementPage {
    pub lines: Vec<StatementLine>,
    /// If there is a next page, pass this code back.
    pub next_cursor: Option<String>,
}

impl StatementPage {
    pub fn new(lines: Vec<StatementLine>, next_cursor: Option<String>) -> Self {
        StatementPage { lines, next_cursor }
    }
}

/// A payment gateway.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PaymentGateway: Send + Sync {
    /// Stable name (`vnpay`, `fake`…): appears in the webhook path and on every payment.
    fn provider(&self) -> &'static str;

    fn capabilities(&self) -> GatewayCapabilities;

    /// Creates a collection. Calling again with the same `payment_id` must return the same transaction (no
    /// new one is created).
    async fn create_payment(
        &self,
        tenant: TenantId,
        intent: &PaymentIntent,
    ) -> Result<GatewayPayment, Error>;

    async fn refund(&self, tenant: TenantId, intent: &RefundIntent)
    -> Result<GatewayRefund, Error>;

    async fn payment_status(
        &self,
        tenant: TenantId,
        provider_ref: &str,
    ) -> Result<GatewayPaymentStatus, Error>;

    /// Verifies the signature over the **raw body** (constant-time comparison) and the replay-protection time
    /// window, and only then reads the content. Failure → [`signature_invalid`]. `async` so the connector can
    /// read the secret of tenant `tenant` (per-tenant config store) — never verify with another tenant's
    /// secret.
    async fn verify_webhook(
        &self,
        tenant: TenantId,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<GatewayNotification, Error>;

    /// The response the partner expects once the webhook is received; `None`: the standard `{"data": …}` envelope.
    fn webhook_ack(&self) -> Option<crate::provider::WebhookAck> {
        None
    }

    /// Statement for the days `[from, to]`.
    async fn statement(
        &self,
        tenant: TenantId,
        from: Date,
        to: Date,
        cursor: Option<String>,
    ) -> Result<StatementPage, Error>;
}

// ---- Chi trả ----------------------------------------------------------------------------------

/// A payout to send to the partner (the marketplace pays a seller from the partner's collection account).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PayoutIntent {
    /// Payout ID in the ledger, sent as the merchant's code: calling again with the same ID does not pay out twice.
    pub payout_id: Uuid,
    /// Recipient (the seller's node ID); the connector looks up the receiving account registered at the
    /// partner itself.
    pub payee: String,
    pub amount: Money,
    /// Short text; contains no personal data.
    pub description: String,
}

impl PayoutIntent {
    pub fn new(
        payout_id: Uuid,
        payee: impl Into<String>,
        amount: Money,
        description: impl Into<String>,
    ) -> Self {
        PayoutIntent { payout_id, payee: payee.into(), amount, description: description.into() }
    }
}

/// Status of a payout at the partner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayoutStatus {
    /// The partner has accepted the order but it is not finished (ask again with [`PayoutGateway::payout_status`]).
    Pending,
    Succeeded,
    Failed {
        reason: String,
    },
}

/// Result of sending a payout.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GatewayPayout {
    pub provider_ref: String,
    pub status: PayoutStatus,
}

impl GatewayPayout {
    pub fn new(provider_ref: impl Into<String>, status: PayoutStatus) -> Self {
        GatewayPayout { provider_ref: provider_ref.into(), status }
    }
}

/// A partner that disburses on behalf of others (licensed): pays sellers from the collection account — the
/// core holds no money. Connectors contribute through `context.contribute::<dyn PayoutGateway>(…)`; tests use
/// `FakePayoutGateway`.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PayoutGateway: Send + Sync {
    /// Stable name (`fake`…), recorded on every payout.
    fn provider(&self) -> &'static str;

    /// Sends a payout order. Calling again with the same `payout_id` must return the same order (no
    /// additional payout).
    async fn payout(&self, tenant: TenantId, intent: &PayoutIntent)
    -> Result<GatewayPayout, Error>;

    /// Status of payout order `provider_ref`.
    async fn payout_status(
        &self,
        tenant: TenantId,
        provider_ref: &str,
    ) -> Result<PayoutStatus, Error>;
}

// ---- Biến động số dư ngân hàng (VietQR tự khớp) -------------------------------------------------

/// Direction of a balance movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankDirection {
    /// Money into the account.
    Credit,
    /// Money out.
    Debit,
}

impl BankDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            BankDirection::Credit => "credit",
            BankDirection::Debit => "debit",
        }
    }
}

/// A balance movement on a seller's / tenant's bank account.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BankTransaction {
    /// Transaction code at the provider (unique): redelivery is not recorded twice.
    pub provider_txn_id: String,
    /// Receiving / paying account number.
    pub account_number: String,
    pub direction: BankDirection,
    /// Always positive.
    pub amount: Money,
    /// Transfer description as the bank wrote it (may be stripped of diacritics, truncated, or padded with
    /// whitespace).
    pub memo: String,
    pub booked_at: Timestamp,
}

impl BankTransaction {
    pub fn new(
        provider_txn_id: impl Into<String>,
        account_number: impl Into<String>,
        direction: BankDirection,
        amount: Money,
        memo: impl Into<String>,
        booked_at: Timestamp,
    ) -> Self {
        BankTransaction {
            provider_txn_id: provider_txn_id.into(),
            account_number: account_number.into(),
            direction,
            amount,
            memo: memo.into(),
            booked_at,
        }
    }
}

/// A bank account whose movements are to be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BankAccountRef {
    /// Bank BIN code (NAPAS), e.g. `970436`.
    pub bank_bin: String,
    pub account_number: String,
}

impl BankAccountRef {
    pub fn new(bank_bin: impl Into<String>, account_number: impl Into<String>) -> Self {
        BankAccountRef { bank_bin: bank_bin.into(), account_number: account_number.into() }
    }
}

/// One pull of movements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BankBatch {
    pub transactions: Vec<BankTransaction>,
    /// Pass this code back next time (store it on the account), even when the page is empty.
    pub next_cursor: Option<String>,
    /// More data remains right after this page.
    pub has_more: bool,
}

impl BankBatch {
    pub fn new(transactions: Vec<BankTransaction>, next_cursor: Option<String>) -> Self {
        BankBatch { transactions, next_cursor, has_more: false }
    }

    #[must_use]
    pub fn more(mut self) -> Self {
        self.has_more = true;
        self
    }
}

/// A balance-movement source: a bank or a statement-reading service (D-194). Money goes straight into the
/// seller's / tenant's account — the platform only reads movements to match payments and never holds money
/// (D-187). Connectors contribute through `context.contribute::<dyn BankFeed>(…)`; tests use `FakeBankFeed`.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait BankFeed: Send + Sync {
    /// Stable name, appears in the webhook path `/api/v1/ledger/bank-feeds/{provider}/{hook}`.
    fn provider(&self) -> &'static str;

    /// Push style: verifies the signature over the **raw body** and only then reads it. Failure →
    /// [`signature_invalid`]. `async` so the connector can read the secret of tenant `tenant`.
    async fn verify_webhook(
        &self,
        tenant: TenantId,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<Vec<BankTransaction>, Error>;

    /// The response the movement source expects once the webhook is received; `None`: the standard envelope.
    fn webhook_ack(&self) -> Option<crate::provider::WebhookAck> {
        None
    }

    /// Pull style: movements after `cursor` (`None` the first time).
    async fn fetch(
        &self,
        tenant: TenantId,
        account: &BankAccountRef,
        cursor: Option<String>,
    ) -> Result<BankBatch, Error>;
}

/// An exchange rate reported by a partner: 1 `base` = `rate` `quote` on date `on`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuotedRate {
    pub base: Currency,
    pub quote: Currency,
    /// Number of major units of `quote` per 1 major unit of `base` (e.g. 1 USD = 25345.5 VND). Never
    /// a floating-point number: read from text, kept as an exact decimal.
    pub rate: rust_decimal::Decimal,
    pub on: Date,
}

impl QuotedRate {
    pub fn new(base: Currency, quote: Currency, rate: rust_decimal::Decimal, on: Date) -> Self {
        QuotedRate { base, quote, rate, on }
    }
}

/// An exchange-rate source (bank, data service — D-237): `ledger` stores the received rates in the tenant's
/// rate table (source `provider:<name>`), next to manually entered rates. Connectors contribute through
/// `context.contribute::<dyn ExchangeRateSource>(…)`; tests use `FakeExchangeRateSource`.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ExchangeRateSource: Send + Sync {
    /// Stable name (`provider:<name>` in the rate's source column).
    fn provider(&self) -> &'static str;

    /// Rates of `base` to every currency this source has, on date `on`. Not to be called inside a write transaction.
    async fn rates(
        &self,
        tenant: TenantId,
        base: Currency,
        on: Date,
    ) -> Result<Vec<QuotedRate>, Error>;
}

/// `payment.signature_invalid` (401): the notification's signature is invalid.
pub fn signature_invalid() -> Error {
    Error::new(
        ErrorKind::Unauthenticated,
        "payment.signature_invalid",
        "The signature of the partner notification is not valid.",
    )
}

/// `payment.provider_unavailable` (503): the partner did not answer.
pub fn provider_unavailable() -> Error {
    Error::unavailable(
        "payment.provider_unavailable",
        "The payment partner is not answering. Try again later.",
    )
}

/// `payment.provider_rejected` (422): the partner rejected it.
pub fn provider_rejected(reason: impl ToString) -> Error {
    Error::new(
        ErrorKind::Validation,
        "payment.provider_rejected",
        "The payment partner refused: {reason}.",
    )
    .with_param("reason", reason)
}

/// `payment.not_supported` (422): the partner does not support this.
pub fn not_supported(provider: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "payment.not_supported",
        "The partner `{provider}` does not support this.",
    )
    .with_param("provider", provider)
}
