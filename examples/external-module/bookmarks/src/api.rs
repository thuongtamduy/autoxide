//! The contract of `bookmarks` for other modules: only what is here may be used from outside
//! (`acme_bookmarks::api::…`). Another module takes the trait when it starts —
//! `context.require::<dyn acme_bookmarks::api::Bookmarks>()?` — and is registered after this one.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

/// Counts bookmarks, e.g. for a dashboard module.
// `#[async_trait]`: Rust cannot yet call an `async fn` of a trait through `dyn Trait`; the macro
// turns it into a function returning a boxed future.
#[async_trait::async_trait]
pub trait Bookmarks: Send + Sync {
    /// How many bookmarks user `owner_id` has in `tenant`.
    async fn count(&self, tenant: &TenantContext, owner_id: Uuid) -> Result<u64, Error>;
}
