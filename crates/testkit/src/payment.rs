//! Cổng thanh toán giả cho test (như `FakeTextModel`): giữ mọi thứ trong bộ nhớ, ký webhook bằng
//! một khoá cố định, cho giả lập đối tác không trả lời và dựng sao kê.

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
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

/// Tên của cổng giả (đường dẫn webhook `/api/v1/ledger/webhooks/fake`).
pub const FAKE_PROVIDER: &str = "fake";
const SECRET: &str = "fake-gateway-secret";
const SIGNATURE: &str = "x-fake-signature";
const TIMESTAMP: &str = "x-fake-timestamp";
/// Cửa sổ chống phát lại của chữ ký, giây.
const WINDOW: i64 = 300;

/// Cổng thanh toán giả. Góp vào app bằng
/// `TestApp::builder(…).contribute::<dyn PaymentGateway>(gateway.clone())`.
#[derive(Default)]
pub struct FakePaymentGateway {
    /// Lượt thu đã tạo, theo ID thanh toán: (mã ở đối tác, số tiền).
    pub payments: Mutex<HashMap<Uuid, (String, Money)>>,
    /// Lượt hoàn đã nhận, theo thứ tự.
    pub refunds: Mutex<Vec<RefundIntent>>,
    /// Số lời gọi `create_payment` / `refund` kế tiếp trả `payment.provider_unavailable`.
    unavailable: AtomicUsize,
    /// Hoàn tiền trả `Pending` (đối tác báo kết quả sau qua webhook) thay vì xong ngay.
    refunds_pending: AtomicBool,
    statement: Mutex<Vec<StatementLine>>,
    /// Không có `split` và `collection_on_behalf` (thử chế độ dòng tiền không hợp với cổng).
    limited: AtomicBool,
}

impl FakePaymentGateway {
    pub fn new() -> Self {
        FakePaymentGateway::default()
    }

    /// `count` lời gọi kế tiếp tới đối tác lỗi như khi đối tác không trả lời.
    pub fn fail_next(&self, count: usize) {
        self.unavailable.store(count, Ordering::SeqCst);
    }

    /// Hoàn tiền chờ webhook thay vì xong ngay.
    pub fn refunds_pending(&self, pending: bool) {
        self.refunds_pending.store(pending, Ordering::SeqCst);
    }

    /// Bỏ khả năng chia tiền tại cổng và thu hộ (mặc định có cả hai).
    pub fn limited(&self, limited: bool) {
        self.limited.store(limited, Ordering::SeqCst);
    }

    /// Đặt sao kê đối tác trả.
    pub fn set_statement(&self, lines: Vec<StatementLine>) {
        if let Ok(mut slot) = self.statement.lock() {
            *slot = lines;
        }
    }

    /// Mã giao dịch ở đối tác của thanh toán `payment_id` (sau `create_payment`).
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

    /// Webhook đã ký (header, thân) mang thông báo `body` (JSON: `event_id`, `kind`
    /// (`payment_succeeded` / `payment_failed` / `refund_succeeded` / `refund_failed`),
    /// `provider_ref`, `payment_id`, `amount` (đơn vị nhỏ nhất), `currency`, `reason`,
    /// `refund_provider_ref`).
    pub fn webhook(&self, body: &Value) -> (Vec<(String, String)>, Vec<u8>) {
        Self::signed(body, Utc::now().timestamp())
    }

    /// Như [`webhook`](Self::webhook) với mốc thời gian tự chọn (thử chống phát lại).
    pub fn signed(body: &Value, timestamp: i64) -> (Vec<(String, String)>, Vec<u8>) {
        let bytes = body.to_string().into_bytes();
        let headers = vec![
            (SIGNATURE.to_owned(), Self::sign(timestamp, &bytes)),
            (TIMESTAMP.to_owned(), timestamp.to_string()),
        ];
        (headers, bytes)
    }

    /// Webhook "đã thu `amount`" cho thanh toán `payment_id`, mã thông báo `event_id`.
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

    fn verify_webhook(
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
        if (Utc::now().timestamp() - timestamp).abs() > WINDOW {
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
        let at = DateTime::<Utc>::from_timestamp(timestamp, 0).unwrap_or_else(Utc::now);
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
        from: NaiveDate,
        to: NaiveDate,
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

/// Kết quả [`FakePayoutGateway`] trả cho lượt chi kế tiếp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FakePayoutOutcome {
    #[default]
    Succeed,
    Fail,
    /// Đối tác nhận lệnh, báo kết quả sau (`payout_status` trả theo [`FakePayoutGateway::settle`]).
    Pending,
}

/// Đối tác chi hộ giả. Góp vào app bằng
/// `TestApp::builder(…).contribute::<dyn PayoutGateway>(payouts.clone())`.
#[derive(Default)]
pub struct FakePayoutGateway {
    /// Lệnh chi đã nhận (mỗi `payout_id` một lần), theo thứ tự.
    pub payouts: Mutex<Vec<PayoutIntent>>,
    outcome: Mutex<FakePayoutOutcome>,
    /// Kết quả cuối của lệnh đang chờ, theo mã ở đối tác.
    finals: Mutex<HashMap<String, PayoutStatus>>,
    unavailable: AtomicUsize,
}

impl FakePayoutGateway {
    pub fn new() -> Self {
        FakePayoutGateway::default()
    }

    /// Kết quả của các lượt chi kế tiếp.
    pub fn outcome(&self, outcome: FakePayoutOutcome) {
        if let Ok(mut slot) = self.outcome.lock() {
            *slot = outcome;
        }
    }

    /// `count` lời gọi `payout` kế tiếp lỗi như khi đối tác không trả lời.
    pub fn fail_next(&self, count: usize) {
        self.unavailable.store(count, Ordering::SeqCst);
    }

    /// Kết thúc lệnh đang chờ của lượt chi `payout_id`: thành công hay thất bại.
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

    /// Số lệnh chi đã nhận.
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

/// Nguồn biến động số dư giả: kiểu kéo trả các giao dịch đã [`push`](Self::push) theo từng tài
/// khoản (mã con trỏ là số giao dịch đã đọc); kiểu đẩy ký thân bằng một khoá cố định. Góp vào app
/// bằng `TestApp::builder(…).contribute::<dyn BankFeed>(feed.clone())`.
#[derive(Default)]
pub struct FakeBankFeed {
    transactions: Mutex<Vec<BankTransaction>>,
    /// Số lời gọi `fetch`.
    pub fetches: AtomicUsize,
}

impl FakeBankFeed {
    pub fn new() -> Self {
        FakeBankFeed::default()
    }

    /// Thêm một biến động (cho kiểu kéo).
    pub fn push(&self, transaction: BankTransaction) {
        if let Ok(mut transactions) = self.transactions.lock() {
            transactions.push(transaction);
        }
    }

    /// Một khoản tiền vào tài khoản `account_number`.
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
            Utc::now(),
        )
    }

    /// Webhook đã ký (header, thân) mang các biến động `transactions`.
    pub fn webhook(transactions: &[BankTransaction]) -> (Vec<(String, String)>, Vec<u8>) {
        let items: Vec<Value> = transactions
            .iter()
            .map(|txn| {
                json!({
                    "id": txn.provider_txn_id, "account": txn.account_number,
                    "direction": txn.direction.as_str(), "amount": txn.amount.minor(),
                    "currency": txn.amount.currency().code(), "memo": txn.memo,
                    "booked_at": txn.booked_at.timestamp(),
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

    fn verify_webhook(
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
                .and_then(|at| DateTime::<Utc>::from_timestamp(at, 0))
                .unwrap_or_else(Utc::now);
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
