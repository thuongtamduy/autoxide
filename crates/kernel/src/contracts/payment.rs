//! Thanh toán qua đối tác có giấy phép (bước 36, D-187): trait chung để module `ledger` gọi cổng
//! thanh toán mà không phụ thuộc đối tác nào. **Core không bao giờ tự giữ tiền**: tiền đi từ người
//! mua tới đối tác (VNPay, MoMo, ngân hàng…), sổ cái chỉ ghi nghĩa vụ và đối soát với sao kê.
//!
//! Bộ nối của một đối tác là một module riêng, chỉ phụ thuộc kernel, đăng ký TRƯỚC `ledger`, góp
//! bằng `context.contribute::<dyn PaymentGateway>(…)` — nhiều đối tác cùng chạy được. Mọi hàm nhận
//! khách (`TenantId`): bộ nối tự đọc thông tin kết nối của khách đó (kho secret theo khách), không
//! đọc biến môi trường cho thứ thuộc về khách. Test dùng `FakePaymentGateway` của testkit.

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use crate::error::{Error, ErrorKind};
use crate::money::Money;
use crate::tenancy::TenantId;

/// Những gì một cổng làm được.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct GatewayCapabilities {
    /// Phương thức nhận được (`card`, `wallet`, `bank_transfer`, `vietqr`…).
    pub methods: Vec<String>,
    pub refunds: bool,
    pub partial_refunds: bool,
    /// Chia tiền ngay tại cổng cho nhiều bên nhận.
    pub split: bool,
    /// Đối tác thu hộ, chi hộ (có giấy phép) cho sàn.
    pub collection_on_behalf: bool,
    /// Có sao kê để đối soát.
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

/// Phần tiền chia cho một bên nhận ngay tại cổng (chỉ khi cổng có `split`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    /// Mã bên nhận ở đối tác.
    pub payee: String,
    pub amount: Money,
}

/// Một lượt thu tiền cần tạo ở đối tác.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PaymentIntent {
    /// ID thanh toán trong sổ, gửi làm mã đơn của merchant: đối tác chống trùng theo mã này, nên
    /// gọi lại cùng ID không tạo hai giao dịch.
    pub payment_id: Uuid,
    pub amount: Money,
    pub method: String,
    /// Chữ ngắn hiện cho người mua; không chứa dữ liệu cá nhân.
    pub description: String,
    /// Trang người mua quay về sau khi trả.
    pub return_url: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub splits: Vec<Split>,
}

impl PaymentIntent {
    pub fn new(
        payment_id: Uuid,
        amount: Money,
        method: impl Into<String>,
        description: impl Into<String>,
        expires_at: DateTime<Utc>,
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

/// Lượt thu đối tác vừa tạo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GatewayPayment {
    /// Mã giao dịch ở đối tác.
    pub provider_ref: String,
    /// Trang thanh toán của đối tác, nếu có.
    pub checkout_url: Option<String>,
    /// Chuỗi QR, nếu đối tác trả.
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

/// Một lượt hoàn tiền cần gửi đối tác.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RefundIntent {
    /// ID lượt hoàn trong sổ (đối tác chống trùng theo mã này).
    pub refund_id: Uuid,
    /// Mã giao dịch thu ở đối tác.
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

/// Trạng thái một lượt hoàn ở đối tác.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefundStatus {
    Pending,
    Succeeded,
    Failed,
}

/// Kết quả gửi lượt hoàn.
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

/// Trạng thái một lượt thu ở đối tác.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayPaymentStatus {
    Pending,
    /// Đã thu `amount` (có thể khác số yêu cầu).
    Succeeded {
        amount: Money,
    },
    Failed {
        reason: String,
    },
    Expired,
}

/// Loại thông báo đối tác gửi qua webhook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationKind {
    PaymentSucceeded,
    PaymentFailed {
        reason: String,
    },
    /// Lượt hoàn (mã ở đối tác) đã xong.
    RefundSucceeded {
        refund_provider_ref: String,
    },
    RefundFailed {
        refund_provider_ref: String,
    },
}

/// Một thông báo đã kiểm chữ ký.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GatewayNotification {
    /// Mã thông báo ở đối tác: gửi lại nhiều lần cùng mã thì chỉ xử lý một lần.
    pub event_id: String,
    pub kind: NotificationKind,
    /// Mã giao dịch thu ở đối tác.
    pub provider_ref: String,
    /// ID thanh toán trong sổ (mã đơn của merchant), nếu đối tác gửi kèm.
    pub payment_id: Option<Uuid>,
    pub amount: Money,
    pub occurred_at: DateTime<Utc>,
}

impl GatewayNotification {
    pub fn new(
        event_id: impl Into<String>,
        kind: NotificationKind,
        provider_ref: impl Into<String>,
        amount: Money,
        occurred_at: DateTime<Utc>,
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

/// Loại dòng sao kê của đối tác.
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

/// Một dòng sao kê.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatementLine {
    /// Mã dòng ở đối tác (duy nhất).
    pub line_id: String,
    /// Mã giao dịch liên quan (thu, hoàn…).
    pub provider_ref: String,
    pub kind: StatementKind,
    /// Số tiền của giao dịch (luôn dương).
    pub amount: Money,
    /// Phí đối tác thu trên dòng này (0 nếu không có).
    pub fee: Money,
    pub booked_on: NaiveDate,
}

impl StatementLine {
    pub fn new(
        line_id: impl Into<String>,
        provider_ref: impl Into<String>,
        kind: StatementKind,
        amount: Money,
        booked_on: NaiveDate,
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

/// Một trang sao kê.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatementPage {
    pub lines: Vec<StatementLine>,
    /// Còn trang sau thì gửi lại mã này.
    pub next_cursor: Option<String>,
}

impl StatementPage {
    pub fn new(lines: Vec<StatementLine>, next_cursor: Option<String>) -> Self {
        StatementPage { lines, next_cursor }
    }
}

/// Một cổng thanh toán.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait PaymentGateway: Send + Sync {
    /// Tên ổn định (`vnpay`, `fake`…): nằm trong đường dẫn webhook và trên mỗi thanh toán.
    fn provider(&self) -> &'static str;

    fn capabilities(&self) -> GatewayCapabilities;

    /// Tạo lượt thu. Gọi lại cùng `payment_id` phải trả cùng giao dịch (không tạo thêm).
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

    /// Kiểm chữ ký trên **thân thô** (so sánh thời gian hằng) và cửa sổ thời gian chống phát lại,
    /// rồi mới đọc nội dung. Sai → [`signature_invalid`].
    fn verify_webhook(
        &self,
        tenant: TenantId,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<GatewayNotification, Error>;

    /// Sao kê các ngày `[from, to]`.
    async fn statement(
        &self,
        tenant: TenantId,
        from: NaiveDate,
        to: NaiveDate,
        cursor: Option<String>,
    ) -> Result<StatementPage, Error>;
}

/// `payment.signature_invalid` (401): chữ ký của thông báo không hợp lệ.
pub fn signature_invalid() -> Error {
    Error::new(
        ErrorKind::Unauthenticated,
        "payment.signature_invalid",
        "The signature of the partner notification is not valid.",
    )
}

/// `payment.provider_unavailable` (503): đối tác không trả lời.
pub fn provider_unavailable() -> Error {
    Error::unavailable(
        "payment.provider_unavailable",
        "The payment partner is not answering. Try again later.",
    )
}

/// `payment.provider_rejected` (422): đối tác từ chối.
pub fn provider_rejected(reason: impl ToString) -> Error {
    Error::new(
        ErrorKind::Validation,
        "payment.provider_rejected",
        "The payment partner refused: {reason}.",
    )
    .with_param("reason", reason)
}

/// `payment.not_supported` (422): đối tác không làm việc này.
pub fn not_supported(provider: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "payment.not_supported",
        "The partner `{provider}` does not support this.",
    )
    .with_param("provider", provider)
}
