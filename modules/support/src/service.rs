//! Service của module `support`: MỌI logic nằm ở đây và các module con (`tickets`, `messages`,
//! `guest`). Handler REST và tool MCP chỉ lấy dữ liệu vào, gọi service — hai đường cho cùng kết quả.
//!
//! Phiếu thuộc một khách (`tenant_id`) và một nút (`org_id`, bước 34). Nhân viên đọc / ghi theo
//! phạm vi permission ở nút của phiếu (`scope_condition`, `require_at`); phiếu ngoài phạm vi báo như
//! không có (404). Người mua chỉ chạm phiếu của chính mình; khách vãng lai chạm một phiếu bằng token.

use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::{OrgId, OrgTree, TenantContext, root_of};
use autoxide_media::api::Media;
use autoxide_notifications::api::Notifier;
use chrono::{DateTime, SubsecRound, Utc};
use uuid::Uuid;

use crate::config::SupportConfig;
use crate::entity::{message, ticket};

/// Một phiếu.
pub(crate) type Ticket = ticket::Model;
/// Một tin nhắn.
pub(crate) type Message = message::Model;

/// Tên module (bảng phụ trợ, throttle, chống spam).
pub(crate) const MODULE: &str = "support";

/// Phần phụ thuộc của service, lấy lúc `start`.
pub(crate) struct Parts {
    pub outbox: Outbox,
    pub guard: Arc<SpamGuard>,
    pub secret: SecretKey,
    /// Gửi mã xác thực và thông báo; `None` khi không cài `notifications`.
    pub notifier: Option<Arc<dyn Notifier>>,
    /// Kiểm tệp đính kèm; `None` khi không cài `media` (gửi tệp thì lỗi).
    pub media: Option<Arc<dyn Media>>,
    /// Số điện thoại đã xác thực của tài khoản (gộp phiếu vãng lai); `None` khi không cài.
    pub accounts: Option<Arc<dyn autoxide_compliance::api::AccountVerification>>,
    /// Quyền hiện tại của người khác (`principal_of`): kiểm người được giao làm được phiếu.
    pub authenticator: Option<Arc<dyn autoxide_kernel::access::Authenticator>>,
    /// Sổ đối tượng: kiểm bản ghi được gắn vào phiếu (D-123); `None` trong test dựng tay.
    pub resources: Option<autoxide_kernel::resource::ResourceDirectory>,
    /// Chuyển phiếu thành khiếu nại người tiêu dùng; `None` khi không cài `compliance`.
    pub complaints: Option<Arc<dyn autoxide_compliance::api::ComplaintIntake>>,
    /// Mô hình soạn bản nháp (module `ai`); `None` khi không cài.
    pub ai: Option<Arc<dyn autoxide_kernel::ai::TextModel>>,
    /// Sổ nguồn câu trả lời từ dữ liệu (điểm cắm `SupportFacts`).
    pub facts: crate::facts::FactRegistry,
    /// Hàng đợi job (trả lời tự động).
    pub jobs: autoxide_kernel::job::JobQueue,
}

// Service không giữ database: database là của khách, đi theo `TenantContext` của từng lời gọi.
pub(crate) struct SupportService {
    pub(crate) parts: Parts,
    pub(crate) config: SupportConfig,
}

impl SupportService {
    pub(crate) fn new(parts: Parts, config: SupportConfig) -> Self {
        SupportService { parts, config }
    }
}

/// Ai đang xem / viết vào phiếu: quyết ghi chú nội bộ có hiện không và ai được làm gì.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Viewer<'a> {
    /// Nhân viên, theo permission ở nút của phiếu.
    Staff(&'a Principal),
    /// Người mua có tài khoản: chỉ phiếu của mình.
    Customer(Uuid),
    /// Người mua vãng lai đã đưa đúng token của phiếu.
    Guest,
    /// Chính module (job, bản nháp AI): thấy như nhân viên, không qua permission.
    Internal,
}

impl Viewer<'_> {
    pub(crate) fn is_staff(&self) -> bool {
        matches!(self, Viewer::Staff(_) | Viewer::Internal)
    }
}

/// Đích kiểm quyền của phiếu: nút của phiếu, người được giao là "chủ" (độ sâu `own`).
pub(crate) fn target(ticket: &Ticket) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(ticket.org_id), ticket.assignee_user_id)
}

/// Nút `org` có trong cây tổ chức của khách (nút gốc luôn có, kể cả khi khách chưa dựng cây).
pub(crate) async fn org_exists(tenant: &TenantContext, org: OrgId) -> Result<bool, Error> {
    if org == root_of(tenant.id()) {
        return Ok(true);
    }
    Ok(!OrgTree::ancestors(tenant.database().read(), tenant.id(), org).await?.is_empty())
}

pub(crate) fn not_found(id: Uuid) -> Error {
    Error::not_found("support.ticket_not_found", "There is no ticket `{id}` you can see.")
        .with_param("id", id)
}

/// Đọc và khoá dòng phiếu `id` tới hết `transaction` ghi (D-181). Mọi đường đọc trạng thái phiếu
/// rồi ghi lại (đổi trạng thái, thêm tin, đếm lần thử mã, trả lời tự động) khoá qua đây: trên
/// Postgres hai transaction không cùng đọc trạng thái cũ rồi ghi đè nhau (phiếu đã đóng bị mở lại,
/// lần thử mã bị đếm mất); SQLite vốn một bên ghi mỗi lúc.
pub(crate) async fn lock_ticket(
    transaction: &sea_orm::DatabaseTransaction,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<Ticket, Error> {
    use autoxide_kernel::tenancy::ScopeExt as _;
    use sea_orm::EntityTrait as _;
    autoxide_kernel::rows::lock_one(transaction, ticket::Entity::find_by_id(id).scoped(tenant))
        .await?
        .ok_or_else(|| not_found(id))
}

/// Lỗi "ngoài phạm vi đọc" thành 404 (không lộ phiếu tồn tại).
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

/// Kiểm chữ bắt buộc: bỏ khoảng trắng hai đầu, 1 tới `max` ký tự.
pub(crate) fn required_text(
    field: &'static str,
    value: &str,
    max: usize,
    problems: &mut Vec<FieldError>,
) -> String {
    let value = value.trim();
    if !(1..=max).contains(&value.chars().count()) {
        problems.push(
            FieldError::new(field, "length", "must be 1 to {max} characters, not only spaces")
                .with_param("max", max),
        );
    }
    value.to_owned()
}

/// Chữ thuần thành HTML an toàn: thoát ký tự đặc biệt, xuống dòng thành `<br>` (như `comments`).
pub(crate) fn body_html(body: &str) -> String {
    let mut html = String::with_capacity(body.len() + 16);
    for character in body.chars() {
        match character {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            '\'' => html.push_str("&#39;"),
            '\n' => html.push_str("<br>"),
            '\r' => {}
            other => html.push(other),
        }
    }
    html
}

/// Che email cho người không phải nhân viên có quyền (`a•••@example.com`).
pub(crate) fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((name, domain)) => {
            let first: String = name.chars().take(1).collect();
            format!("{first}•••@{domain}")
        }
        None => "•••".to_owned(),
    }
}

/// Che số điện thoại: giữ 3 số đầu và 3 số cuối (`090•••567`, như D-202).
pub(crate) fn mask_phone(phone: &str) -> String {
    let digits: Vec<char> = phone.chars().collect();
    if digits.len() <= 6 {
        return "•••".to_owned();
    }
    let head: String = digits[..3].iter().collect();
    let tail: String = digits[digits.len() - 3..].iter().collect();
    format!("{head}•••{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chu_thuan_thanh_html_an_toan() {
        assert_eq!(
            body_html("<b>a</b> & \"x\"\nok"),
            "&lt;b&gt;a&lt;/b&gt; &amp; &quot;x&quot;<br>ok"
        );
    }

    #[test]
    fn che_email_va_so_dien_thoai() {
        assert_eq!(mask_email("an@example.com"), "a•••@example.com");
        assert_eq!(mask_phone("+84901234567"), "+84•••567");
    }
}
