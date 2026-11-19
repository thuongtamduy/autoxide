//! Hợp đồng của module `support` cho module khác (`autoxide_support::api::…`). Module dùng phải
//! đăng ký SAU `support` trong `crates/app/src/modules.rs` và lấy trait lúc khởi động:
//!
//! ```text
//! let intake = context.require::<dyn autoxide_support::api::SupportIntake>()?;
//! ```

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

/// Người hỏi của một phiếu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requester {
    /// Một tài khoản (người mua đã đăng nhập).
    User(Uuid),
    /// Số điện thoại người hỏi đã chứng minh (vd OTP của trang xem đơn, bước 45), dạng E.164.
    VerifiedPhone(String),
    /// Email người hỏi đã chứng minh.
    VerifiedEmail(String),
}

/// Một phiếu module khác mở thay người mua (vd nút "mở hỗ trợ" trên trang đơn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTicket {
    /// Nút nhận phiếu (cửa hàng, gian hàng); nút gốc = ID của khách.
    pub org_id: Uuid,
    pub requester: Requester,
    /// Tên hiển thị của người hỏi (rỗng: "Customer").
    pub requester_name: String,
    /// Mã loại (`order`, `product`, `payment`, `account`, `other` hay loại khách tự khai).
    pub category: String,
    pub subject: String,
    pub body: String,
    /// Bản ghi liên quan, dạng `<kind>:<id>` (vd `orders.order:<id>`).
    pub links: Vec<String>,
    /// Khoá chống trùng của bên gọi: gọi lại cùng `reference` trả phiếu cũ, không tạo phiếu mới.
    pub reference: String,
}

/// Phiếu vừa mở (hay đã có với cùng `reference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRef {
    pub id: Uuid,
    /// Số phiếu hiển thị cho người mua.
    pub number: String,
    /// Token xem phiếu của người mua vãng lai — chỉ có ở lần tạo đầu tiên (database chỉ giữ bản
    /// băm); `None` với người hỏi có tài khoản hay khi gọi lại.
    pub access_token: Option<String>,
}

/// Mở phiếu hỗ trợ thay người mua.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro đổi hàm `async`
// thành hàm trả `Box<dyn Future>`.
#[async_trait::async_trait]
pub trait SupportIntake: Send + Sync {
    /// Tạo phiếu; cùng `reference` → phiếu cũ.
    async fn open(&self, tenant: &TenantContext, ticket: NewTicket) -> Result<TicketRef, Error>;
}

/// Một ý định câu hỏi mà một [`FactSource`] trả lời được bằng dữ liệu chính xác (D-230).
#[derive(Debug, Clone, PartialEq)]
pub struct FactIntent {
    /// `<module>.<ý định>`, vd `shipping.tracking`; duy nhất trong cả chương trình.
    pub intent: &'static str,
    /// Câu tiếng Anh mô tả câu hỏi cho bộ phân loại (dịch qua `text:` của module khai).
    pub description: &'static str,
    /// JSON Schema của tham số bộ phân loại rút từ câu hỏi (vd `{"order_number": …}`).
    pub params: serde_json::Value,
}

/// Nguồn câu trả lời dựng từ dữ liệu chính xác (trạng thái đơn, hành trình vận đơn…). Câu trả lời
/// là mẫu câu i18n của module nguồn điền dữ liệu thật — không bao giờ là chữ do mô hình viết.
#[async_trait::async_trait]
pub trait FactSource: Send + Sync {
    /// Các ý định nguồn này trả lời.
    fn intents(&self) -> Vec<FactIntent>;

    /// Câu trả lời cho `intent` với tham số `params`, bằng ngôn ngữ `locale`. `None` khi không
    /// chắc chắn: không tìm thấy, nhiều kết quả, hay `requester` không chứng minh được là chủ dữ liệu
    /// (vd đơn không thuộc tài khoản hay số điện thoại đã xác thực của người hỏi).
    async fn answer(
        &self,
        tenant: &TenantContext,
        requester: &Requester,
        intent: &str,
        params: &serde_json::Value,
        locale: &str,
    ) -> Result<Option<String>, Error>;
}

/// Điểm cắm cho module đăng ký SAU `support` (vd `shipping`, `orders`): gắn [`FactSource`] lúc
/// `start` bằng `context.require::<dyn SupportFacts>()?.attach(Arc::new(…))`. Không có module nào
/// gắn thì trả lời tự động chỉ dùng câu trả lời mẫu đã duyệt.
pub trait SupportFacts: Send + Sync {
    /// Gắn một nguồn; ý định trùng với nguồn đã gắn, hay không mang tiền tố module → lỗi.
    fn attach(&self, source: std::sync::Arc<dyn FactSource>) -> Result<(), Error>;
}

/// Một thư trả lời nhận được (bộ nhận thư thật — webhook của nhà cung cấp email — là module riêng).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundEmail {
    /// Địa chỉ nhận, dạng `support+<khoá trả lời>@…`.
    pub recipient: String,
    /// Địa chỉ người gửi.
    pub sender: String,
    /// Nội dung chữ thuần của thư (phần trích dẫn thư cũ sẽ bị cắt).
    pub text: String,
}

/// Thư đã đi đâu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundOutcome {
    /// Thành một tin của phiếu.
    Added { ticket_id: Uuid },
    /// Bỏ: không khớp phiếu, người gửi lạ, phiếu đã đóng, hay nút chưa bật nhận thư.
    Dropped,
}

/// Điểm nhận email trả lời vào lại phiếu (tắt mặc định: chính sách `inbound_email` của nút).
#[async_trait::async_trait]
pub trait InboundMail: Send + Sync {
    /// Khoá trả lời của phiếu: địa chỉ `Reply-To` của thư thông báo là `support+<khoá>@<miền>`.
    async fn reply_address_key(
        &self,
        tenant: &TenantContext,
        ticket_id: Uuid,
    ) -> Result<String, Error>;

    /// Nhận một thư; không bao giờ tạo phiếu mới.
    async fn receive(
        &self,
        tenant: &TenantContext,
        mail: InboundEmail,
    ) -> Result<InboundOutcome, Error>;
}

/// Một câu hỏi của người mua cần quyết có trả lời tự động không (phiếu hay chat của bước 61).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundQuestion {
    /// Nút nhận câu hỏi: chính sách trả lời tự động và câu trả lời mẫu của nút này.
    pub org_id: Uuid,
    /// Luồng hỏi đáp: `support.ticket:<id>` hay `conversations.thread:<id>`.
    pub thread: String,
    pub requester: Requester,
    pub text: String,
    /// Ngôn ngữ của người mua (`vi`, `en`).
    pub locale: String,
    /// Số câu trả lời tự động liên tiếp đã gửi trong luồng.
    pub auto_replies_in_row: u32,
}

/// Nguồn của câu trả lời tự động.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoSource {
    /// Câu trả lời mẫu đã duyệt (nguyên văn, đã điền biến).
    Macro(Uuid),
    /// Mẫu câu của một [`FactSource`].
    Fact { intent: String },
}

/// Quyết định cho một câu hỏi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyDecision {
    /// Gửi nguyên văn `text` (gắn nhãn "Trả lời tự động" và nút "Gặp nhân viên").
    Send { text: String, source: AutoSource },
    /// Không đủ điều kiện: để nhân viên trả lời (bên gọi có thể soạn bản nháp AI).
    Draft { text: String },
    /// Chuyển người ngay (rủi ro, từ khoá chặn, quá số lần, tắt).
    Handoff,
}

/// Bộ trả lời tự động (D-230) — `conversations` (bước 61) gọi cho tin chat.
#[async_trait::async_trait]
pub trait AutoResponder: Send + Sync {
    async fn decide(
        &self,
        tenant: &TenantContext,
        question: InboundQuestion,
    ) -> Result<ReplyDecision, Error>;
}
