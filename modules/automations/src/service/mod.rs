//! Service của module `automations`: MỌI logic nằm ở đây; handler REST và tool MCP chỉ lấy dữ liệu
//! vào, gọi service, trả kết quả.
//!
//! - `rules`: tạo / sửa / xoá / bật / tắt luật; `validate`: kiểm luật một lần lúc lưu.
//!
//! Service không giữ database: database là của khách, đi theo `TenantContext` của từng lời gọi;
//! việc không có request (event, job, lịch) mở ngữ cảnh khách qua `DatabaseRegistry`.

pub(crate) mod catalog;
pub(crate) mod rules;
pub(crate) mod runs;
pub(crate) mod validate;

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Authenticator;
use autoxide_kernel::automation::AutomationDirectory;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventCatalog, Outbox};
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::template::TemplateEngine;
use autoxide_kernel::tenancy::DatabaseRegistry;
use uuid::Uuid;

use crate::config::AutomationsConfig;

/// Thứ service cần, lấy lúc `Module::start`.
pub(crate) struct Automations {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub jobs: JobQueue,
    /// Tool và mẫu luật của mọi module (điền sau khi mọi module khởi động).
    pub directory: AutomationDirectory,
    /// Mọi event đã khai (kiểm kích hoạt).
    pub events: EventCatalog,
    pub templates: TemplateEngine,
    /// Quyền HIỆN TẠI của người tạo luật (module xác thực công bố). Không có thì hành động không
    /// chạy được (`automations.owner_unavailable`).
    pub authenticator: Option<Arc<dyn Authenticator>>,
    /// Múi giờ của nút (`organizations.timezone`).
    pub orgs: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    /// Nơi lưu idempotency key chung với REST (module `idempotency`): bước gọi tool không chạy hai
    /// lần.
    pub idempotency: Option<Arc<dyn autoxide_kernel::idempotency::IdempotencyStore>>,
    /// Gửi thông báo (bước `notify`, luật tự tạm dừng).
    pub notifier: Option<Arc<dyn autoxide_notifications::api::Notifier>>,
    /// Số liệu dùng được làm kích hoạt `metric`.
    pub metrics: crate::metrics::MetricSources,
    pub config: AutomationsConfig,
}

impl Automations {
    /// Quyền luật `rule` chạy với: quyền HIỆN TẠI của `run_as`, giới hạn bởi quyền của API key đã
    /// tạo / sửa / bật luật nếu có (D-20261121-automation-credential) — chỉ permission key có, trong
    /// hàng rào nút của key, cùng loại bên làm và ID key (audit thấy key). `None`: không còn người
    /// dùng đó (hay không có module xác thực).
    pub(crate) async fn rule_principal(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
        rule: &crate::entity::rule::Model,
    ) -> Result<Option<autoxide_kernel::access::Principal>, Error> {
        let Some(authenticator) = &self.authenticator else { return Ok(None) };
        let Some(owner) = authenticator.principal_of(tenant, rule.run_as).await? else {
            return Ok(None);
        };
        bounded(owner, rule).map(Some)
    }
}

/// `owner` giới hạn bởi bản chụp quyền của API key trong `rule` (không có thì giữ nguyên).
pub(crate) fn bounded(
    owner: autoxide_kernel::access::Principal,
    rule: &crate::entity::rule::Model,
) -> Result<autoxide_kernel::access::Principal, Error> {
    let Some(raw) = &rule.run_as_limit else { return Ok(owner) };
    let limit: autoxide_kernel::access::PrincipalSnapshot =
        serde_json::from_str(raw).map_err(Error::internal)?;
    let limit = limit.restore();
    let mut bounded = if limit.has_all_permissions() {
        owner
    } else {
        let names = limit.permissions();
        owner.only(names.iter().map(String::as_str))
    };
    if let Some(fence) = limit.fence() {
        bounded = bounded.with_fence(fence);
    }
    // `run_as_api_key` giữ ID credential uỷ quyền; với client OAuth, bản chụp `limit` đã mang grant (loại credential
    // giữ nguyên, D-20261126-mcp-oauth) — không gắn nhầm thành API key.
    if let Some(key) = rule.run_as_api_key
        && bounded.oauth().is_none()
    {
        bounded = bounded.via_api_key(key);
    }
    Ok(bounded.with_actor_kind(limit.actor_kind()))
}

/// Cột `run_as_api_key`, `run_as_limit` cho người đang lưu / bật luật: luôn chụp quyền của người làm lúc
/// này (`run_as_limit`), nên luật không bao giờ có hơn quyền lúc tạo / sửa / bật — kể cả khi tài khoản sau đó
/// được thêm quyền (D-20261126-automation-owner-revoked); credential uỷ quyền (API key hay grant OAuth) thì kèm ID
/// của nó (D-20261126-mcp-oauth).
pub(crate) fn credential(
    principal: &autoxide_kernel::access::Principal,
) -> Result<(Option<Uuid>, Option<String>), Error> {
    let limit = serde_json::to_string(&principal.snapshot()).map_err(Error::internal)?;
    Ok((principal.credential_id(), Some(limit)))
}

/// Bây giờ, cắt tới micro giây (hai database lưu cùng một giá trị — dùng làm phiên bản).
pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}

pub(crate) fn rule_not_found(id: Uuid) -> Error {
    Error::not_found("automations.rule_not_found", "There is no automation rule `{id}`.")
        .with_param("id", id)
}

pub(crate) fn run_not_found(id: Uuid) -> Error {
    Error::not_found("automations.run_not_found", "There is no automation run `{id}`.")
        .with_param("id", id)
}

/// Lệnh chỉ người làm được (không phải AI agent): bật luật agent tạo, duyệt bước rủi ro.
pub(crate) fn human_required() -> Error {
    Error::forbidden(
        "automations.human_required",
        "A person must do this, not an AI agent: sign in, or use an API key of kind `human`.",
    )
}
