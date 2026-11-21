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

mod copy;
pub(crate) mod entity;
pub(crate) mod headers;
mod http;
pub(crate) mod mailbox;
mod migration;
mod partners;
mod promote;
mod service;
mod tools;

use serde::{Deserialize, Serialize};

use crate::access::Permission;
use crate::config::Setting;
use crate::entitlement::Limit;
use crate::error::{Error, ErrorKind};
use crate::event::EventInfo;

pub(crate) use copy::SandboxCatalog;
pub use copy::{
    CopyContext, Promotable, PromoteApply, PromoteRequest, Promoted, SandboxCopy, SandboxOptions,
    copied, link_copy, promoted_target,
};
pub(crate) use mailbox::CreateSandboxMessages;
pub use mailbox::{HeldChannel, HeldMessage, HeldReason, SandboxMessage, hold_back, sandbox_only};
pub(crate) use migration::AddSandbox;
pub(crate) use partners::PartnerGuard;
pub use partners::{SANDBOX_PREFIX, SandboxRecipients};
pub use promote::{
    MAX_PROMOTE_ITEMS, PromotableKind, PromoteInput, PromoteItem, PromoteOutcome, PromoteResult,
    promote_record_not_found,
};
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

pub(crate) const SANDBOX_PROMOTED: EventInfo = EventInfo::new(
    "kernel.sandbox_promoted",
    "Records tried in a sandbox were promoted to the tenant as drafts (`sandbox_id` and how many \
     were `created_draft`, `updated_draft`, `unchanged`, `conflict`).",
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

/// URL webhook mà sandbox được gửi thật (cài đặt của khách, đặt TRONG sandbox): mỗi dòng (hay dấu
/// phẩy) một tiền tố. Mặc định rỗng: mọi webhook của sandbox vào hộp thư giả.
pub const WEBHOOK_URLS: crate::tenancy::TenantSetting = crate::tenancy::TenantSetting::new(
    "kernel.sandbox.webhook_urls",
    "Only in a sandbox: URL prefixes its webhooks may really be sent to, one per line or separated \
     by commas (e.g. `https://hooks.example.com/test/`). Empty by default: every webhook of the \
     sandbox is kept in its message box instead (`GET /api/v1/kernel/sandbox/messages`).",
)
.checked(check_webhook_urls);

fn check_webhook_urls(raw: &str) -> Result<(), crate::error::FieldError> {
    match prefixes(raw)
        .find(|prefix| !(prefix.starts_with("https://") || prefix.starts_with("http://")))
    {
        Some(_) => Err(crate::error::FieldError::new(
            "value",
            "invalid",
            "every prefix must start with `https://` or `http://`",
        )),
        None => Ok(()),
    }
}

/// Webhook tới `url` của khách `tenant` được gửi thật: khách thật luôn được; sandbox chỉ khi `url`
/// bắt đầu bằng một tiền tố trong [`WEBHOOK_URLS`]. Module `webhooks` gọi trước khi gửi.
pub async fn webhook_allowed(
    settings: &dyn crate::tenancy::TenantSettings,
    tenant: &crate::tenancy::TenantContext,
    url: &str,
) -> Result<bool, Error> {
    if !tenant.is_sandbox() {
        return Ok(true);
    }
    let Some(value) = settings.get(tenant, &WEBHOOK_URLS).await? else { return Ok(false) };
    Ok(prefixes(value.as_str()).any(|prefix| url_matches(url, prefix)))
}

fn prefixes(raw: &str) -> impl Iterator<Item = &str> {
    raw.split([',', '\n']).map(str::trim).filter(|prefix| !prefix.is_empty())
}

/// `url` bắt đầu bằng `prefix` và tiền tố kết thúc ở ranh giới (hết chuỗi, `/`, `?`, `#`) — để
/// `https://a.example` không mở cửa cho `https://a.example.evil.com`.
fn url_matches(url: &str, prefix: &str) -> bool {
    let Some(rest) = url.strip_prefix(prefix) else { return false };
    prefix.ends_with('/') || rest.is_empty() || rest.starts_with(['/', '?', '#'])
}

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

/// `tenancy.sandbox_needs_tenant_access` (403): nhân sự nền tảng tạo sandbox (sao cấu hình, sản
/// phẩm) của khách khác mà chưa vào khách đó bằng lần cấp quyền vào hỗ trợ (D-20261121-export-entry).
pub fn sandbox_needs_tenant_access(tenant: uuid::Uuid) -> Error {
    Error::forbidden(
        "tenancy.sandbox_needs_tenant_access",
        "Making a sandbox copies data of the tenant: get a support access grant in force for it, \
         then call with the header `Autoxide-Tenant: {tenant}`.",
    )
    .with_param("tenant", tenant)
}

/// `tenancy.sandbox_forbidden` (403): việc chỉ làm được ở khách thật.
pub fn sandbox_forbidden(action: &str) -> Error {
    Error::forbidden(
        "tenancy.sandbox_forbidden",
        "A sandbox cannot do `{action}`; do it in the real tenant.",
    )
    .with_param("action", action.to_owned())
}

#[cfg(test)]
mod tests {
    use super::url_matches;

    #[test]
    fn tien_to_url_webhook_dung_o_ranh_gioi() {
        assert!(url_matches("https://a.example/hook", "https://a.example"));
        assert!(url_matches("https://a.example/test/1", "https://a.example/test/"));
        assert!(!url_matches("https://a.example.evil.com/x", "https://a.example"));
        assert!(!url_matches("https://b.example/hook", "https://a.example"));
    }
}
