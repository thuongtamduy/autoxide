//! Hợp đồng với nhà cung cấp thoại (bước 61, D-232): gọi cửa hàng từ trình duyệt (WebRTC), gọi lại
//! cho người mua, webhook trạng thái cuộc gọi, bản ghi âm. Bộ nối một nhà cung cấp (Stringee,
//! Twilio…) là module riêng chỉ phụ thuộc kernel, đăng ký TRƯỚC module dùng (`conversations`) và góp
//! bằng `context.contribute::<dyn VoiceProvider>(Arc::new(…))?`. Mọi hàm nhận khách: bộ nối tự đọc
//! thông tin kết nối của khách từ kho secret theo khách (bước 33). Test dùng `FakeVoiceProvider` của
//! `autoxide-testkit`.
//!
//! Số điện thoại đầy đủ chỉ đi tới nhà cung cấp; bên gọi không ghi nó vào log, event hay lỗi (dùng
//! [`PhoneNumber::masked`](crate::sms::PhoneNumber::masked)).

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::{Error, ErrorKind};
use crate::sms::PhoneNumber;
use crate::storage::ByteStream;
use crate::tenancy::TenantId;

/// Nhà cung cấp làm được gì.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VoiceCapabilities {
    /// Người mua gọi được từ trình duyệt / ứng dụng (SDK WebRTC của nhà cung cấp).
    pub browser_calls: bool,
    /// Gọi ra số điện thoại (gọi lại cho người mua).
    pub outbound: bool,
    /// Ghi âm cuộc gọi.
    pub recording: bool,
    /// Phát được câu thông báo ghi âm ở đầu cuộc gọi. Không có thì không được bật ghi âm.
    pub recording_notice: bool,
}

/// Ai cần phiên WebRTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    /// Người mua: chỉ gọi được tới `hotline`.
    Customer,
    /// Nhân viên: nhận cuộc gọi tới hotline và gọi ra, danh tính là `identity`.
    Agent,
}

/// Yêu cầu một phiên WebRTC ngắn hạn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSessionRequest {
    pub role: SessionRole,
    /// Danh tính trong phiên (user id): webhook báo đúng người nghe máy.
    pub identity: String,
    /// Số công khai của cửa hàng.
    pub hotline: PhoneNumber,
    /// Cuộc gọi phía mình (người mua gọi), để nhà cung cấp báo lại; `None` cho phiên nhân viên.
    pub call_id: Option<Uuid>,
    /// Câu thông báo ghi âm (đã dịch) khi cuộc gọi được ghi âm.
    pub recording_notice: Option<String>,
}

/// Phiên WebRTC cho SDK của nhà cung cấp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserSession {
    /// Token ngắn hạn đưa cho SDK; không lưu.
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Một cuộc gọi ra: nối máy nhân viên với số người mua.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundCall {
    /// Mã phía mình: nhà cung cấp dùng để chống trùng (gọi lại cùng mã → cùng cuộc gọi).
    pub call_id: Uuid,
    /// Số người mua thấy: hotline của cửa hàng, không bao giờ là số nhân viên.
    pub from: PhoneNumber,
    pub to: PhoneNumber,
    pub agent_identity: String,
    pub record: bool,
    /// Câu thông báo ghi âm (đã dịch); bắt buộc khi `record`.
    pub recording_notice: Option<String>,
}

/// Cuộc gọi phía nhà cung cấp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCall {
    pub provider_call_id: String,
    pub status: CallStatus,
}

/// Trạng thái cuộc gọi. Thứ tự tiến: `Queued` → `Ringing` → `InProgress` → một trạng thái cuối.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallStatus {
    Queued,
    Ringing,
    InProgress,
    Completed,
    NoAnswer,
    Busy,
    Failed,
    Cancelled,
}

impl CallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CallStatus::Queued => "queued",
            CallStatus::Ringing => "ringing",
            CallStatus::InProgress => "in_progress",
            CallStatus::Completed => "completed",
            CallStatus::NoAnswer => "no_answer",
            CallStatus::Busy => "busy",
            CallStatus::Failed => "failed",
            CallStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "queued" => CallStatus::Queued,
            "ringing" => CallStatus::Ringing,
            "in_progress" => CallStatus::InProgress,
            "completed" => CallStatus::Completed,
            "no_answer" => CallStatus::NoAnswer,
            "busy" => CallStatus::Busy,
            "failed" => CallStatus::Failed,
            "cancelled" => CallStatus::Cancelled,
            _ => return None,
        })
    }

    /// Trạng thái cuối: không đổi nữa.
    pub fn is_final(self) -> bool {
        self.rank() == 3
    }

    /// Bậc tiến của trạng thái: webhook tới sai thứ tự không được lùi bậc.
    pub fn rank(self) -> u8 {
        match self {
            CallStatus::Queued => 0,
            CallStatus::Ringing => 1,
            CallStatus::InProgress => 2,
            _ => 3,
        }
    }
}

/// Một thông báo trạng thái từ webhook đã kiểm chữ ký.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceNotification {
    /// Mã event phía nhà cung cấp: chống nhận trùng.
    pub provider_event_id: String,
    pub provider_call_id: String,
    /// Mã phía mình khi nhà cung cấp báo lại (`OutboundCall::call_id`, `BrowserSessionRequest::call_id`).
    pub call_id: Option<Uuid>,
    pub status: CallStatus,
    pub agent_identity: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub duration_seconds: Option<u32>,
    pub provider_recording_id: Option<String>,
}

/// Một nhà cung cấp thoại.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait VoiceProvider: Send + Sync {
    /// Tên ổn định (`stringee`, `twilio`, `fake`…): nằm trong đường dẫn webhook và trên mỗi cuộc gọi.
    fn provider(&self) -> &'static str;

    fn capabilities(&self) -> VoiceCapabilities;

    /// Token ngắn hạn cho SDK WebRTC.
    async fn browser_session(
        &self,
        tenant: TenantId,
        request: &BrowserSessionRequest,
    ) -> Result<BrowserSession, Error>;

    /// Gọi ra. Gọi lại cùng `call_id` phải trả cùng cuộc gọi (không gọi thêm).
    async fn place_call(
        &self,
        tenant: TenantId,
        call: &OutboundCall,
    ) -> Result<ProviderCall, Error>;

    /// Kiểm chữ ký trên **thân thô** (so sánh thời gian hằng) và cửa sổ chống phát lại, rồi mới đọc
    /// nội dung. Sai → [`signature_invalid`].
    fn verify_webhook(
        &self,
        tenant: TenantId,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<Vec<VoiceNotification>, Error>;

    /// Tải bản ghi âm.
    async fn fetch_recording(
        &self,
        tenant: TenantId,
        provider_recording_id: &str,
    ) -> Result<ByteStream, Error>;

    /// Xoá bản ghi ở nhà cung cấp (sau khi đã lưu về). Xoá cái đã xoá thì thành công.
    async fn delete_recording(
        &self,
        tenant: TenantId,
        provider_recording_id: &str,
    ) -> Result<(), Error>;
}

/// `voice.signature_invalid` (401): chữ ký webhook sai hay quá cửa sổ thời gian.
pub fn signature_invalid() -> Error {
    Error::new(
        ErrorKind::Unauthenticated,
        "voice.signature_invalid",
        "The signature of the voice provider notification is not valid.",
    )
}

/// `voice.provider_unavailable` (503): nhà cung cấp không trả lời.
pub fn provider_unavailable() -> Error {
    Error::unavailable(
        "voice.provider_unavailable",
        "The voice provider is not answering. Try again later.",
    )
}

#[cfg(test)]
mod tests {
    use super::CallStatus;

    #[test]
    fn trang_thai_cuoi_va_bac_tien() {
        for status in [CallStatus::Completed, CallStatus::NoAnswer, CallStatus::Busy] {
            assert!(status.is_final());
            assert_eq!(CallStatus::parse(status.as_str()), Some(status));
        }
        assert!(CallStatus::Ringing.rank() < CallStatus::InProgress.rank());
        assert!(!CallStatus::Queued.is_final());
    }
}
