//! The [`Authenticator`] trait: how the kernel asks a module who is calling.

use uuid::Uuid;

use super::Principal;
use crate::error::Error;
use crate::tenancy::{TenantContext, TenantHint};

/// Authenticates a request: reads the value of the `Authorization` header and returns the calling party. A
/// module publishes this trait at startup (`context.provide::<dyn Authenticator>(…)`); the kernel attaches it
/// to every request.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Authenticator: Send + Sync {
    /// `authorization` is the whole header value (`None` when absent); `hint` is the tenant the request
    /// targets (domain, `Autoxide-Tenant` header) — credentials issued before step 33 do not carry a tenant,
    /// and the authenticator uses it to know which database to look in. The returned error is usually 401
    /// (missing, wrong, expired) or 403 (account locked).
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        hint: &TenantHint,
    ) -> Result<Principal, Error>;

    /// A user authenticated in the home tenant (`principal.tenant_id()`) asks to enter tenant `target` (header
    /// `Autoxide-Tenant`, step 34): delegation from the parent tenant, or time-limited support access. Returns
    /// a new `Principal` in tenant `target` (permissions by route of entry, [`Principal::with_access`]) or
    /// `None` when there is no route in. Recomputed on every request: revocation and expiry take effect
    /// on the next request. Default: no route in.
    async fn enter(
        &self,
        principal: &Principal,
        target: &TenantContext,
    ) -> Result<Option<Principal>, Error> {
        let _ = (principal, target);
        Ok(None)
    }

    /// `true`: only [`enter`](Self::enter) decides who enters another tenant — platform staff with
    /// `kernel.tenants.operate` must also have a support access grant (D-190, D-313). Default
    /// `false`: the kernel's platform staff rule (step 33) still applies.
    fn decides_entry(&self) -> bool {
        false
    }

    /// Current permissions of the user `user_id` in tenant `tenant`, without a request (step 51 P10, P11): to
    /// check that the person assigned a task has the source's permission, and to list people who can be
    /// granted a permission. `None`: there is no such active user (or the authenticator does not know).
    /// Default: `None`.
    async fn principal_of(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Option<Principal>, Error> {
        let _ = (tenant, user_id);
        Ok(None)
    }
}
