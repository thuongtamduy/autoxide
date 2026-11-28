//! Sample data for new tenants (step 35): what a module contributes with
//! `context.contribute::<dyn SampleData>(…)` in `start`.

use std::sync::Arc;

use crate::error::Error;

use super::{TenantContext, TenantSeed};

/// Seeds a module's sample data for a tenant provisioned under a plan with `sample_data` (step 35). Must be safe
/// to rerun and must not create duplicates. A module contributes it in `start`:
/// `context.contribute::<dyn SampleData>(Arc::new(samples))?`.
//
// Trait riêng (không dùng lại `TenantSeed`): `contribute` chọn chỗ nhận theo KIỂU trait, nên dữ liệu ban đầu và dữ
// liệu mẫu phải là hai kiểu khác nhau. `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SampleData: Send + Sync {
    /// Writes the sample data into `tenant`.
    async fn sample(&self, tenant: &TenantContext) -> Result<(), Error>;
}

/// A contributed [`SampleData`] seen as a [`TenantSeed`], the shape the provisioning code runs.
pub(crate) struct SampleSeed(pub(crate) Arc<dyn SampleData>);

#[async_trait::async_trait]
impl TenantSeed for SampleSeed {
    async fn seed(&self, tenant: &TenantContext) -> Result<(), Error> {
        self.0.sample(tenant).await
    }
}

/// The old `Module::sample_data` (a [`TenantSeed`]) seen as a [`SampleData`], so both paths are collected alike.
pub(crate) struct SeedAsSample(pub(crate) Arc<dyn TenantSeed>);

#[async_trait::async_trait]
impl SampleData for SeedAsSample {
    async fn sample(&self, tenant: &TenantContext) -> Result<(), Error> {
        self.0.seed(tenant).await
    }
}
