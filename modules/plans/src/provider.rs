//! `dyn Entitlements` module `plans` công bố cho kernel: quyền dùng gộp của mỗi khách (có bộ nhớ
//! đệm), và (phần D) giữ chỗ / trả lại hạn mức.

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::entitlement::{Entitlements, Limit, TenantEntitlements, TenantPlanView};
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

    async fn reserve_part(
        &self,
        tenant: TenantId,
        limit: &Limit,
        part: &str,
        amount: i64,
        reference: &str,
    ) -> Result<(), Error> {
        self.0.reserve_part(tenant, limit, part, amount, reference).await
    }

    async fn confirm_part(
        &self,
        tenant: TenantId,
        limit: &Limit,
        part: &str,
        reference: &str,
    ) -> Result<(), Error> {
        self.0.confirm_part(tenant, limit, part, reference).await
    }

    async fn release_part(
        &self,
        tenant: TenantId,
        limit: &Limit,
        part: &str,
        reference: &str,
    ) -> Result<(), Error> {
        self.0.release_part(tenant, limit, part, reference).await
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

    async fn describe_tenants(
        &self,
        tenants: &[TenantId],
        subscription: bool,
        usage: bool,
    ) -> Result<BTreeMap<TenantId, TenantPlanView>, Error> {
        let service = &self.0;
        let mut subscriptions = BTreeMap::new();
        if subscription {
            for row in service.subscriptions_of(tenants).await? {
                let tenant = TenantId::from_uuid(row.subscription.tenant_id);
                let value = serde_json::to_value(crate::http::SubscriptionResponse::from(row))
                    .map_err(|error| Error::internal(format!("subscription to JSON: {error}")))?;
                subscriptions.insert(tenant, value);
            }
        }
        let mut uses = BTreeMap::new();
        if usage {
            let mut used = service.usage_of_many(tenants, crate::service::now()).await?;
            for tenant in tenants {
                // Quyền dùng từng khách qua bộ nhớ đệm (đọc lại tối đa vài câu mỗi khách).
                let snapshot = service.snapshot(*tenant).await?;
                let used = used.remove(tenant).unwrap_or_default();
                let list = crate::http::limit_uses_of(service, &snapshot, &used);
                let value = serde_json::to_value(list)
                    .map_err(|error| Error::internal(format!("usage to JSON: {error}")))?;
                uses.insert(*tenant, value);
            }
        }
        let mut found = BTreeMap::new();
        for tenant in tenants {
            let view = TenantPlanView::new(subscriptions.remove(tenant), uses.remove(tenant));
            if view != TenantPlanView::default() {
                found.insert(*tenant, view);
            }
        }
        Ok(found)
    }
}
