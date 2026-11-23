//! Test tenants (sandbox, step 38, D-232, D-20261118-step38-sandbox).
//!
//! A tenant owner creates a trial copy carrying the real tenant's CONFIGURATION (no personal data): one
//! `kernel_tenants` row of kind `sandbox`, `sandbox_of` = the source tenant, `parent_id` NULL (not a child
//! tenant, not in anyone's `descendants`), on the same placement as the source tenant, so data never leaves
//! the location the tenant chose; isolation is by `tenant_id`. A sandbox expires by itself
//! (`AUTOXIDE_TENANCY_SANDBOX_DAYS`), can be extended, and is deleted through the tenant erasure flow. Staff of
//! the source tenant enter the sandbox by delegation (the `auth` module listens to `kernel.sandbox_created`),
//! with the `Autoxide-Tenant` header as when entering a child tenant.
//!
//! Every sandbox response carries `Autoxide-Sandbox: true` and `X-Robots-Tag: noindex, nofollow`
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

/// Tenant kind (column `kernel_tenants.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantKind {
    /// A real tenant.
    Production,
    /// A trial copy of a real tenant (`sandbox_of`).
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

/// View a tenant's sandboxes (and the fake mailbox when inside a sandbox).
pub const SANDBOXES_READ: Permission = Permission::new(
    "kernel.sandboxes.read",
    "See the sandboxes (test copies) of your tenant, the kinds of records you can promote from \
     them and, inside a sandbox, the messages it held back.",
);

/// Create, extend and delete sandboxes, and push what was tried to the real tenant.
pub const SANDBOXES_MANAGE: Permission = Permission::new(
    "kernel.sandboxes.manage",
    "Create, extend and delete sandboxes of your tenant (test copies with its configuration, no \
     personal data), and promote what was tried there to your tenant as drafts.",
)
.with_risk(crate::access::Risk::Destructive);

/// Number of sandboxes a tenant holds at once. 1 when the plan sets nothing, including tenants with no
/// subscription (each sandbox costs resources, D-232).
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

/// Number of days a sandbox lives if not extended.
pub const SANDBOX_DAYS: Setting = Setting::new(
    "AUTOXIDE_TENANCY_SANDBOX_DAYS",
    "Days a sandbox (test copy of a tenant) lives unless extended (1 to 365); extending sets it \
     again from now.",
)
.with_default("30");

/// Cap of the `products` option when creating a sandbox.
pub const SANDBOX_MAX_PRODUCTS: Setting = Setting::new(
    "AUTOXIDE_TENANCY_SANDBOX_MAX_PRODUCTS",
    "Most products a new sandbox may copy from its tenant (0 to 1000); the `products` asked when \
     creating it must not be more.",
)
.with_default("50");

/// Webhook URLs that a sandbox may really send to (a tenant setting, set INSIDE the sandbox): one prefix per
/// line (or comma). Empty by default: every sandbox webhook goes to the fake mailbox.
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

/// Whether a webhook to `url` of tenant `tenant` is really sent: a real tenant always is; a sandbox only when
/// `url` starts with a prefix in [`WEBHOOK_URLS`]. The `webhooks` module calls it before sending.
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

/// `url` starts with `prefix` and the prefix ends at a boundary (end of string, `/`, `?`, `#`), so that
/// `https://a.example` does not open the door for `https://a.example.evil.com`.
fn url_matches(url: &str, prefix: &str) -> bool {
    let Some(rest) = url.strip_prefix(prefix) else { return false };
    prefix.ends_with('/') || rest.is_empty() || rest.starts_with(['/', '?', '#'])
}

/// Job kind that copies the configuration and then activates the sandbox (`unique_key = "sandbox_copy:{sandbox}"`).
pub(crate) const SANDBOX_COPY: &str = "kernel.sandbox_copy";
/// Schedule that deletes expired sandboxes (every hour, control database).
pub(crate) const EXPIRE_SANDBOXES: &str = "kernel.expire_sandboxes";

/// `tenancy.sandbox_not_found` (404): the caller's tenant has no such sandbox.
pub fn sandbox_not_found(id: uuid::Uuid) -> Error {
    Error::not_found("tenancy.sandbox_not_found", "There is no sandbox `{id}` of this tenant.")
        .with_param("id", id)
}

/// `tenancy.sandbox_nested` (422): creating a sandbox from a sandbox.
pub fn sandbox_nested() -> Error {
    Error::new(
        ErrorKind::Validation,
        "tenancy.sandbox_nested",
        "A sandbox cannot be made from a sandbox; make it from the real tenant.",
    )
}

/// `tenancy.sandbox_needs_tenant_access` (403): platform staff create a sandbox (copying configuration,
/// products) of another tenant without having entered that tenant through a support-access grant
/// (D-20261121-export-entry).
pub fn sandbox_needs_tenant_access(tenant: uuid::Uuid) -> Error {
    Error::forbidden(
        "tenancy.sandbox_needs_tenant_access",
        "Making a sandbox copies data of the tenant: get a support access grant in force for it, \
         then call with the header `Autoxide-Tenant: {tenant}`.",
    )
    .with_param("tenant", tenant)
}

/// `tenancy.sandbox_forbidden` (403): an action that can only be done at a real tenant.
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
