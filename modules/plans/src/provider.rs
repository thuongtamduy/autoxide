//! `dyn Entitlements` module `plans` công bố cho kernel: quyền dùng gộp của mỗi khách (có bộ nhớ
//! đệm), và (phần D) giữ chỗ / trả lại hạn mức.

use std::sync::Arc;

use autoxide_kernel::entitlement::{Entitlements, Limit, TenantEntitlements};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantId;

use crate::service::PlansService;

pub(crate) struct Provider(pub Arc<PlansService>);

#[async_trait::async_trait]
impl Entitlements for Provider {
    async fn snapshot(&self, tenant: TenantId) -> Result<Arc<TenantEntitlements>, Error> {
        self.0.snapshot(tenant).await
    }

    async fn reserve(
        &self,
        tenant: TenantId,
        limit: &Limit,
        amount: i64,
        reference: &str,
    ) -> Result<(), Error> {
        self.0.reserve(tenant, limit, amount, reference).await
    }

    async fn confirm(&self, tenant: TenantId, limit: &Limit, reference: &str) -> Result<(), Error> {
        self.0.confirm(tenant, limit, reference).await
    }

    async fn release(&self, tenant: TenantId, limit: &Limit, reference: &str) -> Result<(), Error> {
        self.0.release(tenant, limit, reference).await
    }

    async fn record_usage(
        &self,
        tenant: TenantId,
        limit: &Limit,
        quantity: i64,
        reference: &str,
    ) -> Result<(), Error> {
        self.0.record_usage(tenant, limit, quantity, reference).await
    }

    async fn tenants_with_status(&self, status: &str) -> Result<Vec<TenantId>, Error> {
        self.0.tenants_with_status(status).await
    }

    async fn tenants_near_limit(&self) -> Result<Vec<TenantId>, Error> {
        self.0.tenants_near_limit().await
    }
}
