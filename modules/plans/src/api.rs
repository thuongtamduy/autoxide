//! Contract of the `plans` module for other modules and for the binary.
//!
//! Entitlements do NOT go through here: other modules ask `autoxide_kernel::entitlement::EntitlementsHandle`
//! (obtained with `StartContext::entitlements`); the `plans` module is only the provider. What is here is the
//! provisioning of new tenants by plan (step 35 item 5), for the `autoxide tenant create … --plan` command.

use autoxide_kernel::error::Error;
use uuid::Uuid;

/// A new tenant to provision by plan.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProvisionRequest {
    pub slug: String,
    pub name: String,
    pub plan_id: Uuid,
    pub admin_email: String,
    pub price_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    /// Placement name; `None`: `main` (plan `shared`).
    pub placement: Option<String>,
    pub admin_name: Option<String>,
    /// `None`: follow the plan's `sample_data`.
    pub sample_data: Option<bool>,
    /// Grant a trial if the plan has one.
    pub trial: bool,
}

impl ProvisionRequest {
    pub fn new(
        slug: impl Into<String>,
        name: impl Into<String>,
        plan_id: Uuid,
        admin_email: impl Into<String>,
    ) -> Self {
        ProvisionRequest {
            slug: slug.into(),
            name: name.into(),
            plan_id,
            admin_email: admin_email.into(),
            price_id: None,
            parent_id: None,
            placement: None,
            admin_name: None,
            sample_data: None,
            trial: true,
        }
    }
}

/// Result of one provisioning run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProvisioningStatus {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    /// Most recent step completed (`done` when finished).
    pub step: String,
    /// `pending`, `running`, `failed`, `done`.
    pub status: String,
}

impl ProvisioningStatus {
    pub fn new(id: Uuid, tenant_id: Option<Uuid>, step: String, status: String) -> Self {
        ProvisioningStatus { id, tenant_id, step, status }
    }
}

/// Provisions a new tenant by plan: create the tenant → assign the plan → sample data → invite the
/// administrator. Does not check permissions — the caller is responsible (the CLI command runs with the
/// operator's authority).
#[async_trait::async_trait]
pub trait Provisioning: Send + Sync {
    /// Runs all steps immediately (not through the job queue). A failed step: error
    /// `plans.provisioning_failed`; the run is recorded and can be resumed with
    /// `POST /api/v1/plans/provisionings/{id}/retry`.
    async fn provision(&self, request: ProvisionRequest) -> Result<ProvisioningStatus, Error>;
}
