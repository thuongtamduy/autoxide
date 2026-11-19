//! Service của module `conversations`: MỌI logic nằm ở đây và các module con (`threads`,
//! `messages`, `assist`, `abuse`, `privacy`). Handler REST và tool MCP chỉ lấy dữ liệu vào, gọi
//! service — hai đường cho cùng kết quả.
//!
//! Cuộc trò chuyện thuộc một khách (`tenant_id`) và một nút (`org_id`, bước 34). Nhân viên đọc / ghi
//! theo phạm vi permission ở nút của cuộc trò chuyện (`scope_condition`, `require_at`); ngoài phạm vi
//! báo như không có (404). Người mua chỉ chạm cuộc trò chuyện của chính mình.

use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, PostingVerification, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::tenancy::{OrgId, OrgTree, TenantContext, root_of};
use autoxide_media::api::Media;
use autoxide_notifications::api::Notifier;
use chrono::{DateTime, SubsecRound, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::ConversationsConfig;
use crate::entity::{message, thread};
use crate::settings::Settings;

/// Một cuộc trò chuyện.
pub(crate) type Thread = thread::Model;
/// Một tin nhắn.
pub(crate) type Message = message::Model;

/// Tên module (bảng phụ trợ, throttle).
pub(crate) const MODULE: &str = "conversations";

/// Phần phụ thuộc của service, lấy lúc `start`.
pub(crate) struct Parts {
    pub outbox: Outbox,
    pub jobs: autoxide_kernel::job::JobQueue,
    pub settings: Settings,
    /// Hỏi tài khoản đã xác thực để đăng nội dung chưa (NĐ 147/2024, D-202).
    pub verification: PostingVerification,
    /// Thông báo tin chưa đọc; `None` khi không cài `notifications`.
    pub notifier: Option<Arc<dyn Notifier>>,
    /// Kiểm ảnh đính kèm; `None` khi không cài `media` (gửi ảnh thì lỗi).
    pub media: Option<Arc<dyn Media>>,
    /// Chuyển cuộc trò chuyện thành phiếu; `None` khi không cài `support`.
    pub intake: Option<Arc<dyn autoxide_support::api::SupportIntake>>,
    /// Trả lời tự động của `support` (D-230); `None` khi không cài `support`.
    pub responder: Option<Arc<dyn autoxide_support::api::AutoResponder>>,
    /// Mô hình soạn bản nháp (module `ai`); `None` khi không cài.
    pub ai: Option<Arc<dyn autoxide_kernel::ai::TextModel>>,
    /// Quyền hiện tại của người khác (`principal_of`): kiểm người được giao làm được cuộc trò chuyện.
    pub authenticator: Option<Arc<dyn autoxide_kernel::access::Authenticator>>,
    /// Sổ đối tượng: kiểm bản ghi được hỏi (D-123); `None` trong test dựng tay.
    pub resources: Option<autoxide_kernel::resource::ResourceDirectory>,
}

// Service không giữ database: database là của khách, đi theo `TenantContext` của từng lời gọi.
pub(crate) struct ConversationsService {
    pub(crate) parts: Parts,
    pub(crate) config: ConversationsConfig,
}

impl ConversationsService {
    pub(crate) fn new(parts: Parts, config: ConversationsConfig) -> Self {
        ConversationsService { parts, config }
    }
}

/// Ai đang xem / viết vào cuộc trò chuyện.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Viewer<'a> {
    /// Nhân viên, theo permission ở nút của cuộc trò chuyện.
    Staff(&'a Principal),
    /// Người mua: chỉ cuộc trò chuyện của mình.
    Customer(Uuid),
}

impl Viewer<'_> {
    pub(crate) fn user_id(&self) -> Uuid {
        match self {
            Viewer::Staff(principal) => principal.user_id(),
            Viewer::Customer(user) => *user,
        }
    }

    pub(crate) fn is_staff(&self) -> bool {
        matches!(self, Viewer::Staff(_))
    }
}

/// Người viết tin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorKind {
    Customer,
    Agent,
    System,
}

impl AuthorKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            AuthorKind::Customer => "customer",
            AuthorKind::Agent => "agent",
            AuthorKind::System => "system",
        }
    }
}

pub(crate) const OPEN: &str = "open";
pub(crate) const CLOSED: &str = "closed";

/// Đích kiểm quyền của cuộc trò chuyện: nút của nó, người được giao là "chủ" (độ sâu `own`).
pub(crate) fn target(thread: &Thread) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(thread.org_id), thread.assignee_user_id)
}

/// Người nhận event của cuộc trò chuyện qua stream: người mua và người được giao.
pub(crate) fn audience(thread: &Thread) -> Vec<Uuid> {
    let mut people: Vec<Uuid> =
        thread.customer_user_id.into_iter().chain(thread.assignee_user_id).collect();
    people.dedup();
    people
}

/// Nút `org` có trong cây tổ chức của khách (nút gốc luôn có, kể cả khi khách chưa dựng cây).
pub(crate) async fn org_exists(tenant: &TenantContext, org: OrgId) -> Result<bool, Error> {
    if org == root_of(tenant.id()) {
        return Ok(true);
    }
    Ok(!OrgTree::ancestors(tenant.database().read(), tenant.id(), org).await?.is_empty())
}

pub(crate) fn not_found(id: Uuid) -> Error {
    Error::not_found(
        "conversations.thread_not_found",
        "There is no conversation `{id}` you can see.",
    )
    .with_param("id", id)
}

/// Lỗi "ngoài phạm vi đọc" thành 404 (không lộ cuộc trò chuyện tồn tại).
pub(crate) fn hidden(result: Result<(), Error>, id: Uuid) -> Result<(), Error> {
    result.map_err(|error| if error.code() == "org_out_of_scope" { not_found(id) } else { error })
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên — cắt trước để hai
/// database trả cùng một giá trị.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Gom lỗi đầu vào và báo một lần (D-083).
pub(crate) fn finish(problems: Vec<FieldError>) -> Result<(), Error> {
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

/// Những gì audit cần biết về một cuộc trò chuyện — không nội dung.
pub(crate) fn snapshot(thread: &Thread) -> Value {
    json!({
        "id": thread.id,
        "org_id": thread.org_id,
        "customer_user_id": thread.customer_user_id,
        "subject": thread.subject,
        "status": thread.status,
        "assignee_user_id": thread.assignee_user_id,
        "unread_customer": thread.unread_customer,
        "unread_staff": thread.unread_staff,
        "needs_agent": thread.needs_agent,
        "auto_reply_paused": thread.auto_reply_paused,
        "ticket_id": thread.ticket_id,
    })
}

/// Event về tin nhắn: không mang nội dung.
pub(crate) fn message_snapshot(message: &Message) -> Value {
    json!({
        "id": message.id,
        "thread_id": message.thread_id,
        "author_kind": message.author_kind,
        "author_user_id": message.author_user_id,
        "auto_reply": message.auto_reply,
        "call_id": message.call_id,
    })
}

/// Mảng ID ảnh lưu dạng JSON.
pub(crate) fn media_of(message: &Message) -> Vec<Uuid> {
    serde_json::from_str(&message.media_ids).unwrap_or_default()
}
