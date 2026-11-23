//! Contract of the `auth` module for other modules: only what is here may be used from
//! outside (`autoxide_auth::api::…`).
//!
//! Other modules obtain the trait at startup:
//!
//! ```text
//! let users = context.require::<dyn autoxide_auth::api::Users>()?;
//! ```
//!
//! That module must be registered AFTER `auth` in `crates/app/src/modules.rs`.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

/// A user account. Never carries a password or its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    /// Normalized email (surrounding whitespace removed, lowercased). Unique.
    pub email: String,
    /// Display name.
    pub name: String,
    /// When the account was locked; `None` means active.
    pub disabled_at: Option<Timestamp>,
    /// When the account owner confirmed (by mail) that `email` is theirs; `None`: not verified.
    pub email_verified_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Staff (has roles) or a tenant's buyer (no permissions) — step 34, D-309.
    pub realm: autoxide_kernel::access::Realm,
    /// Preferred language (`en`, `vi`) for mail and deferred notifications; `None`: follow the request (D-308).
    pub locale: Option<String>,
    /// Preferred IANA time zone; `None`: not set.
    pub time_zone: Option<String>,
}

impl User {
    /// The account is still active (not locked).
    pub fn is_active(&self) -> bool {
        self.disabled_at.is_none()
    }
}

/// Looks up users. Other modules use this trait to check that a user ID exists
/// (no cross-module foreign key — D-025) and to display names.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro
// này đổi hàm `async` thành hàm trả về `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Users: Send + Sync {
    /// The user with ID `id` of tenant `tenant`, or `None` if there is none.
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<User>, Error>;
}

/// A tenant's administrator: an active user with a role carrying every permission (role `admin`).
/// The `notifications` module uses it to send platform notifications (overdue subscription, nearing a limit…)
/// to the administrators of every tenant (step 35, D-348).
#[async_trait::async_trait]
pub trait TenantAdmins: Send + Sync {
    /// Active administrators of tenant `tenant`, ordered by ID.
    async fn admins(&self, tenant: &TenantContext) -> Result<Vec<User>, Error>;
}

/// Data to create an account.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub name: String,
    /// `None`: the account has no password yet and cannot sign in with a password.
    pub password: Option<String>,
    /// Assign the `admin` role (every permission).
    pub admin: bool,
}

/// Creates an account. The binary uses this trait for the `autoxide user create` command; it does not
/// check permissions — the caller is responsible (the CLI command runs with the authority of the
/// server operator).
#[async_trait::async_trait]
pub trait Accounts: Send + Sync {
    /// Creates an account. Errors: invalid or already used email (`auth.email_taken`), password that
    /// fails the length rules (`validation_failed`).
    async fn create(&self, tenant: &TenantContext, account: NewAccount) -> Result<User, Error>;
}

/// A new invitation for the `autoxide user invite` command and for tenant provisioning (step 35).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewInvitation {
    pub email: String,
    pub name: Option<String>,
    /// Names of the tenant's roles assigned when the invitee accepts.
    pub roles: Vec<String>,
    /// Branch of the roles; `None`: the root branch.
    pub org_id: Option<Uuid>,
}

impl NewInvitation {
    pub fn new(email: impl Into<String>, roles: Vec<String>) -> Self {
        NewInvitation { email: email.into(), name: None, roles, org_id: None }
    }

    pub fn at(mut self, org_id: Uuid) -> Self {
        self.org_id = Some(org_id);
        self
    }

    /// Suggested display name for the invitee.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// The invitation just created; the mail goes out within a few minutes (job).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InvitationSummary {
    pub id: Uuid,
    pub email: String,
    pub expires_at: Timestamp,
}

impl InvitationSummary {
    pub fn new(id: Uuid, email: impl Into<String>, expires_at: Timestamp) -> Self {
        InvitationSummary { id, email: email.into(), expires_at }
    }
}

/// Invites a user by email (D-312). Does not check permissions — the caller is responsible (the CLI command
/// runs with the operator's authority, platform provisioning).
#[async_trait::async_trait]
pub trait Invitations: Send + Sync {
    /// Invites `invitation.email` to tenant `tenant`. Errors: invalid email, unknown role
    /// (`validation_failed`).
    async fn invite(
        &self,
        tenant: &TenantContext,
        invitation: NewInvitation,
    ) -> Result<InvitationSummary, Error>;
}

/// Tasks for the server operator when no one can sign in to do them through the API. The binary uses it for the
/// `autoxide user reset-mfa` command; does not check permissions — the CLI command runs with the operator's
/// authority (D-159).
#[async_trait::async_trait]
pub trait AccountRecovery: Send + Sync {
    /// Turns off two-step verification (TOTP and recovery codes) of the account with email `email`, as an
    /// administrator does through `DELETE /api/v1/auth/users/{id}/mfa`: for the sole admin who lost their
    /// device. Errors: no such account (`auth.account_not_found`).
    async fn reset_mfa(&self, tenant: &TenantContext, email: &str) -> Result<User, Error>;

    /// Removes every passkey of the account with email `email`, as an administrator does through
    /// `DELETE /api/v1/auth/users/{id}/passkeys`: for the sole admin who lost their device (then signs in
    /// again with a password). Returns the account and the number of passkeys removed. Errors:
    /// `auth.account_not_found`.
    async fn remove_passkeys(
        &self,
        tenant: &TenantContext,
        email: &str,
    ) -> Result<(User, u64), Error>;
}

/// A one-time code was sent to a phone number (step 47, sign-in by phone number).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneCodeSent {
    /// ID of the code; the user submits back the same code they received.
    pub id: Uuid,
    pub expires_at: Timestamp,
}

/// Sends and verifies one-time codes over SMS for phone-number sign-in (step 47, D-20261120-phone-accounts).
/// `compliance` implements it (holding the rules for accepted numbers, send limits, the SMS sender) and plugs
/// it in with [`AuthExtensions::set_phone_codes`] because it registers after `auth`.
#[async_trait::async_trait]
pub trait PhoneCodes: Send + Sync {
    /// Sends a code to number `phone` (not normalized). Errors when the number is not accepted, too many sends, or SMS fails.
    async fn send(
        &self,
        tenant: &TenantContext,
        phone: &str,
        ip: Option<std::net::IpAddr>,
    ) -> Result<PhoneCodeSent, Error>;

    /// Verifies code `code` of `id`: correct, unexpired, unused → the verified number (single use).
    async fn verify(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        code: &str,
    ) -> Result<autoxide_kernel::sms::PhoneNumber, Error>;

    /// Records the number just verified as the number of account `user_id` (so other modules learn of it
    /// through `AccountVerification`). Ignored, without error, when the tenant's rules do not allow adding an
    /// account that uses the number.
    async fn record(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        phone: &autoxide_kernel::sms::PhoneNumber,
    ) -> Result<(), Error>;
}

/// Extension point of `auth` for modules registered later (like `OrderExtensions` of `orders`).
pub trait AuthExtensions: Send + Sync {
    /// Plugs in the sender / verifier of codes for phone-number sign-in. Plugging a second time: error.
    fn set_phone_codes(&self, codes: std::sync::Arc<dyn PhoneCodes>) -> Result<(), Error>;
}
