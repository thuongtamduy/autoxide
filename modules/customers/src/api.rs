//! Contract of the `customers` module for other modules (steps 48–49, `wholesale`, `analytics`…): only
//! what is here is used from outside (`autoxide_customers::api::…`). This file does not reference
//! the module's internals.
//!
//! A module registered AFTER `customers` gets the trait at startup:
//!
//! ```text
//! let customers = context.require::<dyn autoxide_customers::api::Customers>()?;
//! ```
//!
//! `customers` plugs customer groups into `orders` itself (`OrderExtensions::set_buyer_groups`), so `orders`
//! does not need to call this trait.

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Names of the events `customers` emits.
pub mod events {
    pub const PROFILE_CREATED: &str = "customers.profile_created";
    pub const PROFILE_UPDATED: &str = "customers.profile_updated";
    pub const PROFILE_LINKED: &str = "customers.profile_linked";
    pub const GROUP_MEMBERSHIP_CHANGED: &str = "customers.group_membership_changed";
}

/// Summary of a customer profile for other modules. Masked contact details (D-226): never a full phone number
/// or email.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CustomerSummary {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    /// The node where the customer registered / first bought.
    pub org_id: OrgId,
    pub full_name: Option<String>,
    /// `09•• ••• 412`.
    pub phone_masked: Option<String>,
    /// `n•••@gmail.com`.
    pub email_masked: Option<String>,
    /// Current membership tier.
    pub tier: Option<String>,
    pub group_ids: Vec<Uuid>,
    /// Total spent: Σ goods value of completed orders − refunds (D-226). `None` when there is no purchase yet.
    pub total_spent: Option<Money>,
    pub orders_count: i64,
}

/// Reads customer profiles.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này đổi hàm
// `async` thành hàm trả `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Customers: Send + Sync {
    /// The profile linked to buyer account `user_id`, or `None` (none is created).
    async fn find_by_user(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Option<CustomerSummary>, Error>;

    /// IDs of the customer groups of account `user_id` (empty when there is no profile yet).
    async fn groups_of_user(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Vec<Uuid>, Error>;
}
