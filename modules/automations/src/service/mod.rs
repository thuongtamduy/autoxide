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

use std::sync::Arc;

use autoxide_kernel::access::Authenticator;
use autoxide_kernel::automation::AutomationDirectory;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventCatalog, Outbox};
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::template::TemplateEngine;
use autoxide_kernel::tenancy::DatabaseRegistry;
use chrono::{DateTime, SubsecRound, Utc};
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

/// Bây giờ, cắt tới micro giây (hai database lưu cùng một giá trị — dùng làm phiên bản).
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
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
