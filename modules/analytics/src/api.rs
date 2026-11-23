//! Contract of the `analytics` module for other modules (registered AFTER `analytics`):
//! `context.require::<dyn Analytics>()`. Example: `catalog` asks for "trending" (most-viewed objects),
//! `customers` asks for a person's "recently viewed products" (available only when that person consented to
//! per-person tracking).

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use uuid::Uuid;

/// Purpose in the `compliance` consent ledger that allows linking analytics events to a user.
pub const PERSONAL_PURPOSE: &str = "analytics.personal";

/// Event kind "page view" (the website also sends `object` when the page is an object).
pub const PAGE_VIEW: &str = "analytics.page_view";

/// An object and the number of times it was viewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectViews {
    /// `<kind>:<id>`, vd `catalog.product:0190…`.
    pub object: String,
    /// The `<id>` part.
    pub id: String,
    /// Number of events that refer to the object.
    pub views: i64,
    /// Total visitors per day.
    pub visitors: i64,
}

// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Analytics: Send + Sync {
    /// Objects of kind `kind` (e.g. `catalog.product`) most viewed on website `site` (`None`: the tenant's
    /// websites as a whole) in the last `days` days (1–400), most first, at most `limit`
    /// (1–100). Reads the aggregated counts, not raw events.
    async fn top_objects(
        &self,
        tenant: &TenantContext,
        site: Option<OrgId>,
        kind: &str,
        days: u32,
        limit: u32,
    ) -> Result<Vec<ObjectViews>, Error>;

    /// Objects of kind `kind` that user `user_id` recently viewed, newest first, without duplicates, at most
    /// `limit` (1–100). Empty when that person did not consent to per-person tracking (or withdrew).
    async fn recently_viewed(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        kind: &str,
        limit: u32,
    ) -> Result<Vec<String>, Error>;
}
