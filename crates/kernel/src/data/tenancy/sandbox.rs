//! Khách thử nghiệm (sandbox, bước 38, D-232, D-20261118-step38-sandbox).
//!
//! Chủ khách tạo một bản thử mang CẤU HÌNH của khách thật (không mang dữ liệu cá nhân): một dòng
//! `kernel_tenants` loại `sandbox`, `sandbox_of` = khách gốc, `parent_id` NULL (không phải khách con,
//! không nằm trong `descendants` của ai), cùng placement với khách gốc — dữ liệu không rời vị trí
//! khách đã chọn; cách ly bằng `tenant_id`. Sandbox tự hết hạn (`AUTOXIDE_TENANCY_SANDBOX_DAYS`),
//! gia hạn được, xoá theo luồng xoá khách. Nhân viên của khách gốc vào sandbox bằng uỷ quyền (module
//! `auth` nghe `kernel.sandbox_created`), header `Autoxide-Tenant` như vào khách con.
//!
//! Mọi response của sandbox mang `Autoxide-Sandbox: true` và `X-Robots-Tag: noindex, nofollow`
//! (`headers.rs`).

pub(crate) mod entity;
pub(crate) mod headers;
mod http;
mod migration;
mod service;
mod tools;

use serde::{Deserialize, Serialize};

use crate::access::Permission;
use crate::config::Setting;
use crate::entitlement::Limit;
use crate::error::{Error, ErrorKind};
use crate::event::EventInfo;

pub(crate) use migration::AddSandbox;
pub(crate) use service::Wiring;
pub use service::{NewSandbox, Sandbox, SandboxCreated, TenantSandboxes};

/// Loại khách (cột `kernel_tenants.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantKind {
    /// Khách thật.
    Production,
    /// Bản thử của một khách thật (`sandbox_of`).
    Sandbox,
}

impl TenantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TenantKind::Production => "production",
            TenantKind::Sandbox => "sandbox",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "production" => Some(TenantKind::Production),
            "sandbox" => Some(TenantKind::Sandbox),
            _ => None,
        }
    }
}

/// Xem sandbox của khách (và hộp thư giả khi ở trong sandbox).
pub const SANDBOXES_READ: Permission = Permission::new(
    "kernel.sandboxes.read",
    "See the sandboxes (test copies) of your tenant, the kinds of records you can promote from \
     them and, inside a sandbox, the messages it held back.",
);

/// Tạo, gia hạn, xoá sandbox và đẩy thứ đã thử sang khách thật.
pub const SANDBOXES_MANAGE: Permission = Permission::new(
    "kernel.sandboxes.manage",
    "Create, extend and delete sandboxes of your tenant (test copies with its configuration, no \
     personal data), and promote what was tried there to your tenant as drafts.",
)
.with_risk(crate::access::Risk::Destructive);

/// Số sandbox một khách giữ cùng lúc. Gói không đặt thì 1 — kể cả khách không có thuê bao (mỗi
/// sandbox tốn tài nguyên, D-232).
pub const SANDBOXES_LIMIT: Limit = Limit::count(
    "kernel.sandboxes",
    "Sandboxes (test copies) a tenant can keep at the same time; 1 when the plan does not say.",
);

pub(crate) const SANDBOX_CREATED: EventInfo = EventInfo::new(
    "kernel.sandbox_created",
    "A sandbox of the tenant is ready (`sandbox_id`, `expires_at`, `role_id`).",
);
pub(crate) const SANDBOX_EXTENDED: EventInfo = EventInfo::new(
    "kernel.sandbox_extended",
    "A sandbox of the tenant got a new expiry date (`sandbox_id`, `expires_at`).",
);
pub(crate) const SANDBOX_DELETED: EventInfo = EventInfo::new(
    "kernel.sandbox_deleted",
    "A sandbox of the tenant was deleted, by hand or because it expired (`sandbox_id`).",
);

/// Số ngày một sandbox sống nếu không gia hạn.
pub const SANDBOX_DAYS: Setting = Setting::new(
    "AUTOXIDE_TENANCY_SANDBOX_DAYS",
    "Days a sandbox (test copy of a tenant) lives unless extended (1 to 365); extending sets it \
     again from now.",
)
.with_default("30");

/// Trần của tuỳ chọn `products` khi tạo sandbox.
pub const SANDBOX_MAX_PRODUCTS: Setting = Setting::new(
    "AUTOXIDE_TENANCY_SANDBOX_MAX_PRODUCTS",
    "Most products a new sandbox may copy from its tenant (0 to 1000); the `products` asked when \
     creating it must not be more.",
)
.with_default("50");

/// Loại job sao cấu hình rồi kích hoạt sandbox (`unique_key = "sandbox_copy:{sandbox}"`).
pub(crate) const SANDBOX_COPY: &str = "kernel.sandbox_copy";
/// Lịch xoá sandbox hết hạn (mỗi giờ, database điều khiển).
pub(crate) const EXPIRE_SANDBOXES: &str = "kernel.expire_sandboxes";

/// `tenancy.sandbox_not_found` (404): không có sandbox này của khách người gọi.
pub fn sandbox_not_found(id: uuid::Uuid) -> Error {
    Error::not_found("tenancy.sandbox_not_found", "There is no sandbox `{id}` of this tenant.")
        .with_param("id", id)
}

/// `tenancy.sandbox_nested` (422): tạo sandbox từ một sandbox.
pub fn sandbox_nested() -> Error {
    Error::new(
        ErrorKind::Validation,
        "tenancy.sandbox_nested",
        "A sandbox cannot be made from a sandbox; make it from the real tenant.",
    )
}

/// `tenancy.sandbox_forbidden` (403): việc chỉ làm được ở khách thật.
pub fn sandbox_forbidden(action: &str) -> Error {
    Error::forbidden(
        "tenancy.sandbox_forbidden",
        "A sandbox cannot do `{action}`; do it in the real tenant.",
    )
    .with_param("action", action.to_owned())
}
