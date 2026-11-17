//! Cổng thanh toán giả cho test (như `FakeTextModel`): giữ mọi thứ trong bộ nhớ, ký webhook bằng
//! một khoá cố định, cho giả lập đối tác không trả lời và dựng sao kê.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{
    GatewayCapabilities, GatewayNotification, GatewayPayment, GatewayPaymentStatus, GatewayRefund,
    NotificationKind, PaymentGateway, PaymentIntent, RefundIntent, RefundStatus, StatementLine,
    StatementPage, provider_unavailable, signature_invalid,
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
        GatewayCapabilities::new(&["card", "wallet", "vietqr"]).with_refunds(true).with_statements()
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
