//! Khách của người gọi (`GET /api/v1/kernel/tenant`, tool `kernel.current_tenant`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{TenancyMode, TenantAdmin, TenantContext};

/// The tenant of the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CurrentTenant {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub status: String,
    /// Read-only: data can be read but every change is refused with `403 tenant_read_only`.
    /// Always present.
    #[serde(default)]
    #[schema(required = true)]
    pub read_only: bool,
    /// `single` (one tenant) or `multi` (many tenants on one installation).
    #[schema(example = "single")]
    pub mode: String,
    /// Whether this is the platform tenant (the operator's own).
    pub platform: bool,
    /// `production` (a real tenant) or `sandbox` (a test copy: partners are simulated, mail and
    /// webhooks do not leave, nothing done here counts in the real tenant).
    #[serde(default = "production")]
    #[schema(example = "production")]
    pub kind: String,
    /// Sandbox only: the real tenant it was made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_of: Option<Uuid>,
    /// Sandbox only: when it is deleted unless extended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_expires_at: Option<DateTime<Utc>>,
}

fn production() -> String {
    "production".to_owned()
}

impl TenantAdmin {
    /// Khách của người gọi.
    pub fn current(&self, tenant: &TenantContext) -> CurrentTenant {
        CurrentTenant {
            id: tenant.id().as_uuid(),
            slug: tenant.slug().to_owned(),
            name: tenant.name().to_owned(),
            status: tenant.status().as_str().to_owned(),
            read_only: tenant.read_only(),
            mode: match self.registry().mode() {
                TenancyMode::Single => "single",
                TenancyMode::Multi => "multi",
            }
            .to_owned(),
            platform: tenant.is_platform(),
            kind: tenant.kind().as_str().to_owned(),
            sandbox_of: tenant.sandbox_of().map(|of| of.as_uuid()),
            sandbox_expires_at: tenant.sandbox_expires_at(),
        }
    }
}
