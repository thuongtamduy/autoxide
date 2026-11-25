//! Service của module `bookings`: MỌI logic nằm ở đây và ở các file con (`setup`, `openings`,
//! `booking`…). Handler REST và tool MCP chỉ lấy dữ liệu vào, gọi service, trả envelope.
//!
//! Dữ liệu theo khách (bước 33) và theo nút (bước 34): mọi truy vấn lọc `.scoped(tenant)`; lệnh ghi
//! kiểm `require_write_at` TRONG transaction ghi; bản ghi ngoài phạm vi đọc báo như không có (404).

use autoxide_kernel::access::{OrgTarget, Principal, Scope};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

use crate::settings::Settings;

/// Phụ thuộc lấy lúc `start`: bắt buộc thì có, tuỳ chọn thì `None` khi không có module cung cấp.
pub(crate) struct BookingsService {
    pub outbox: Outbox,
    pub settings: Settings,
    pub jobs: autoxide_kernel::job::JobQueue,
    pub databases: autoxide_kernel::tenancy::DatabaseRegistry,
    pub ledger: Option<std::sync::Arc<dyn autoxide_ledger::api::Ledger>>,
    pub notifier: Option<std::sync::Arc<dyn autoxide_notifications::api::Notifier>>,
    pub sms: Option<std::sync::Arc<dyn autoxide_kernel::sms::SmsSender>>,
    /// Cấu hình theo khách (mẫu câu SMS mang mã, D-20261125-sms-templates).
    pub tenant_settings: std::sync::Arc<dyn autoxide_kernel::tenancy::TenantSettings>,
    pub secret: Option<autoxide_kernel::secret::SecretKey>,
}

/// Bây giờ, cắt tới micro giây: Postgres lưu tới micro giây, SQLite lưu nguyên. Cắt trước để hai
/// database trả về cùng một giá trị.
pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}

/// Nút của bản ghi mới khi người tạo không nêu: gốc nếu có `permission` cho cả khách; nút duy nhất
/// nếu mọi grant ở một nút; nhiều nút thì phải chọn (`bookings.org_required`).
pub(crate) fn default_org(
    tenant: &TenantContext,
    principal: &Principal,
    permission: &autoxide_kernel::access::Permission,
) -> Result<OrgId, Error> {
    let grants = principal.grants(permission);
    if grants.is_empty() || grants.iter().any(|grant| grant.scope == Scope::Tenant) {
        return Ok(root_of(tenant.id()));
    }
    let first = grants[0].org;
    if grants.iter().all(|grant| grant.org == first) {
        return Ok(first);
    }
    Err(Error::new(
        ErrorKind::Validation,
        "bookings.org_required",
        "You work in several organization units: give `org_id`.",
    )
    .with_field(FieldError::new("org_id", "required", "choose the organization unit")))
}

/// Đích kiểm quyền của bản ghi ở nút `org`.
pub(crate) fn at(org: Uuid) -> OrgTarget {
    OrgTarget::new(OrgId::from_uuid(org), None)
}

/// Lỗi "ngoài phạm vi" của lệnh đọc thành `not_found` (không lộ bản ghi tồn tại).
pub(crate) fn hidden(
    result: Result<(), Error>,
    not_found: impl FnOnce() -> Error,
) -> Result<(), Error> {
    result.map_err(|error| if error.code() == "org_out_of_scope" { not_found() } else { error })
}

/// Kiểm "giá trị phải nằm trong [min, max] và là bội số của `step`", gom vào `problems`.
pub(crate) fn check_minutes(
    problems: &mut Vec<FieldError>,
    field: &str,
    value: i32,
    min: i32,
    max: i32,
    step: i32,
) {
    if value < min || value > max || value % step != 0 {
        problems.push(
            FieldError::new(field, "range", "must be {min} to {max} minutes, in steps of {step}")
                .with_param("min", min)
                .with_param("max", max)
                .with_param("step", step),
        );
    }
}

/// Chữ ngắn: bỏ khoảng trắng hai đầu, `min`..=`max` ký tự.
pub(crate) fn check_text(
    problems: &mut Vec<FieldError>,
    field: &str,
    value: &str,
    min: usize,
    max: usize,
) {
    let length = value.trim().chars().count();
    if length < min || length > max {
        problems.push(
            FieldError::new(field, "length", "must be {min} to {max} characters")
                .with_param("min", min)
                .with_param("max", max),
        );
    }
}

/// Báo mọi vấn đề của đầu vào một lần (D-083).
pub(crate) fn finish(problems: Vec<FieldError>) -> Result<(), Error> {
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}
