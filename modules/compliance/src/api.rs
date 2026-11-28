//! Contract of the `compliance` module for other modules: the KYC gate, the legal-document acceptance gate,
//! the consent ledger, and legal rules with effective dates. Other modules get them at startup:
//! `context.require::<dyn KycGate>()`… (registered AFTER `compliance`).
//!
//! Legal basis (check against the original text / a person with legal expertise): E-Commerce Law
//! 122/2025/QH15 Article 17 (identification of sellers, livestreamers, affiliate marketing, from 01/7/2026);
//! Law on Electronic Transactions 20/2023/QH15 (accepting electronic terms); Law on Personal Data Protection
//! 91/2025/QH15 + Decree 356/2025 (purpose-based consent, withdrawal).

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext, TenantId};
use autoxide_kernel::time::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// A fake consent ledger for other modules' tests (feature `testing`).
#[cfg(feature = "testing")]
pub mod testing;

/// Activities that need identification first (E-Commerce Law 2025 Article 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = ComplianceActivity)]
pub enum Activity {
    Sell,
    Livestream,
    Affiliate,
}

impl Activity {
    pub fn as_str(self) -> &'static str {
        match self {
            Activity::Sell => "sell",
            Activity::Livestream => "livestream",
            Activity::Affiliate => "affiliate",
        }
    }
}

/// The subject of an identification profile or of a document acceptance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subject {
    /// A user of the tenant.
    User(Uuid),
    /// An organization node (seller, branch).
    Org(OrgId),
    /// An email with no account yet (e.g. someone signing up as a new tenant accepts the terms of service).
    Email(String),
    /// A code the calling module sets when no other subject exists yet (e.g. `plans.signup:<id>`): KYC before
    /// the tenant exists.
    Reference(String),
}

impl Subject {
    /// (`subject_kind`, `subject_id`) stored in the table.
    pub fn columns(&self) -> (&'static str, String) {
        match self {
            Subject::User(id) => ("user", id.to_string()),
            Subject::Org(org) => ("org", org.as_uuid().to_string()),
            Subject::Email(email) => ("email", email.trim().to_lowercase()),
            Subject::Reference(reference) => ("reference", reference.clone()),
        }
    }
}

/// KYC status of a subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceKycStatus)]
pub struct KycStatus {
    /// `none` (no profile), `draft`, `pending`, `needs_info`, `verified`, `rejected`, `expired` or
    /// `revoked`.
    pub status: String,
    pub profile_id: Option<Uuid>,
    pub verified_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
}

/// Identification gate.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait KycGate: Send + Sync {
    /// Ok when the rule in force does not require identification for `activity`, or the subject has a
    /// `verified` profile that has not expired. Otherwise `compliance.kyc_required` (403).
    async fn require_verified(
        &self,
        tenant: TenantId,
        subject: Subject,
        activity: Activity,
    ) -> Result<(), Error>;

    async fn status(&self, tenant: TenantId, subject: Subject) -> Result<KycStatus, Error>;

    /// Opens (or returns) the identification profile of `subject` and starts a verification round at the
    /// provider; returns a page / token for the user. Used when there is no account yet (e.g. a new tenant
    /// signing up).
    ///
    /// `return_url` is trusted as given: the calling module is responsible for it, and must pass only an
    /// address from the server's configuration (such as `AUTOXIDE_PLANS_SIGNUP_RETURN_URL`) or one it has
    /// checked itself — never a string an API caller sent unchecked. (`return_url` sent over this module's REST
    /// and MCP is checked against the tenant's own addresses.)
    async fn start(
        &self,
        tenant: TenantId,
        subject: Subject,
        party: autoxide_kernel::identity::PartyType,
        return_url: Option<String>,
    ) -> Result<KycSession, Error>;
}

/// A started verification: where the person goes next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceKycSession)]
pub struct KycSession {
    pub profile_id: Uuid,
    pub attempt_id: Uuid,
    /// `redirect` (open `url`), `sdk` (start the mobile SDK with `token`) or `pending` (nothing to
    /// do; the result comes later).
    pub kind: String,
    pub url: Option<String>,
    pub token: Option<String>,
    pub expires_at: Option<Timestamp>,
}

/// How far an account is verified for posting content; `identity` includes `phone`.
//
// Mức xác thực của một tài khoản (NĐ 147/2024/NĐ-CP — cần đối chiếu văn bản gốc). `Identity` bao
// `Phone`: thứ tự khai báo là thứ tự mạnh dần, so sánh bằng `>=`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[schema(as = ComplianceVerificationLevel)]
pub enum VerificationLevel {
    /// Not verified.
    None,
    /// A Vietnamese mobile number was confirmed with a one-time code.
    Phone,
    /// The identity was verified (KYC profile `verified` and not expired).
    Identity,
}

impl VerificationLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            VerificationLevel::None => "none",
            VerificationLevel::Phone => "phone",
            VerificationLevel::Identity => "identity",
        }
    }

    /// Read from the policy value `posting.<place>` (`none` / `phone` / `identity`).
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "none" => Some(VerificationLevel::None),
            "phone" => Some(VerificationLevel::Phone),
            "identity" => Some(VerificationLevel::Identity),
            _ => None,
        }
    }
}

/// A place where users post content; a tenant sets the level required for each place through the policy `posting.<place>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostingSurface {
    Comments,
    Reviews,
    Livestream,
}

impl PostingSurface {
    /// Policy key of the posting place, e.g. `posting.comments`.
    pub fn policy_key(self) -> &'static str {
        match self {
            PostingSurface::Comments => "posting.comments",
            PostingSurface::Reviews => "posting.reviews",
            PostingSurface::Livestream => "posting.livestream",
        }
    }
}

/// Content posting gate: compares the level the tenant set for a posting place with the account's level.
#[async_trait::async_trait]
pub trait PostingGate: Send + Sync {
    /// A `None` principal = anonymous visitor, who passes only when the policy is `none`. When the level the
    /// tenant set is not met → `compliance.verification_required` (403, with `level`).
    async fn check(
        &self,
        tenant: TenantId,
        principal: Option<&autoxide_kernel::access::Principal>,
        surface: PostingSurface,
    ) -> Result<(), Error>;
}

/// Account verification state.
#[async_trait::async_trait]
pub trait AccountVerification: Send + Sync {
    async fn level(&self, tenant: TenantId, user_id: Uuid) -> Result<VerificationLevel, Error>;

    /// The verified number (e.g. for `notifications` to send SMS / ZNS). None yet → `None`.
    async fn verified_phone(
        &self,
        tenant: TenantId,
        user_id: Uuid,
    ) -> Result<Option<autoxide_kernel::sms::PhoneNumber>, Error>;
}

/// Kind of legal document.
pub const DOCUMENT_KINDS: [&str; 6] = [
    "terms_of_use",
    "privacy_policy",
    "general_conditions",
    "platform_rules",
    "seller_agreement",
    "developer_agreement",
];

/// Legal document acceptance gate.
#[async_trait::async_trait]
pub trait AcceptanceGate: Send + Sync {
    /// Whether the subject has accepted the current version that must be accepted of document `kind` (tenant
    /// level); if not → `compliance.acceptance_required` (403). A tenant with no document of this kind: Ok.
    async fn require_accepted(
        &self,
        tenant: TenantId,
        subject: Subject,
        kind: &str,
    ) -> Result<(), Error>;

    /// Records acceptance of the current version of document `kind` for `subject` (calling again is
    /// harmless). Returns the ID of the version accepted; a tenant with no document of this kind: `None`.
    async fn accept_current(
        &self,
        tenant: TenantId,
        subject: Subject,
        kind: &str,
        channel: &str,
    ) -> Result<Option<Uuid>, Error>;
}

/// Who consents.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConsentSubject {
    User(Uuid),
    Email(String),
    /// E.164 number.
    Phone(String),
}

/// What the consent is for: purpose, scope (`""` = all), channel (`""` = all).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceConsentKey)]
pub struct ConsentKey {
    /// e.g. `marketing`.
    pub purpose: String,
    /// `""` (everything) or e.g. `newsletter.list:<id>`.
    #[serde(default)]
    pub scope: String,
    /// `""` (every channel), `email`, `sms`, `zns`, `push` or `in_app`.
    #[serde(default)]
    pub channel: String,
}

impl ConsentKey {
    pub fn new(
        purpose: impl Into<String>,
        scope: impl Into<String>,
        channel: impl Into<String>,
    ) -> Self {
        ConsentKey { purpose: purpose.into(), scope: scope.into(), channel: channel.into() }
    }
}

/// Evidence of consent: the wording the user saw, the wording version, where they consented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceConsentEvidence)]
pub struct ConsentEvidence {
    pub text: String,
    pub text_version: String,
    /// Where: `web_form`, `checkout`, `import`…
    pub source: String,
    pub ip_fingerprint: Option<String>,
}

/// A consent, as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceConsent)]
pub struct Consent {
    pub id: Uuid,
    pub purpose: String,
    pub scope: String,
    pub channel: String,
    /// `granted` or `withdrawn`.
    pub state: String,
    pub granted_at: Option<Timestamp>,
    pub withdrawn_at: Option<Timestamp>,
    /// The email or phone number (`null` for a user, or after erasure).
    pub subject_value: Option<String>,
}

/// Consent ledger by purpose.
#[async_trait::async_trait]
pub trait ConsentLedger: Send + Sync {
    /// Records a consent; calling again with the same `source_ref` → records nothing more, returns the current state.
    async fn grant(
        &self,
        tenant: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        evidence: ConsentEvidence,
        source_ref: &str,
    ) -> Result<Consent, Error>;

    async fn withdraw(
        &self,
        tenant: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        source: &str,
        source_ref: &str,
    ) -> Result<Consent, Error>;

    /// Whether a consent is in force at exactly this key or a broader key (an empty `scope` / `channel`
    /// covers the children), and there is no withdrawal at a broader key.
    async fn is_granted(
        &self,
        tenant: TenantId,
        subject: &ConsentSubject,
        key: &ConsentKey,
    ) -> Result<bool, Error>;

    async fn granted_many(
        &self,
        tenant: TenantId,
        subjects: &[ConsentSubject],
        key: &ConsentKey,
    ) -> Result<Vec<bool>, Error>;

    async fn get(&self, tenant: TenantId, consent_id: Uuid) -> Result<Option<Consent>, Error>;
}

/// A legal rule in force.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceRule)]
pub struct Rule {
    pub jurisdiction: String,
    pub key: String,
    pub version: i64,
    pub effective_from: Timestamp,
    pub effective_to: Option<Timestamp>,
    #[schema(value_type = Object)]
    pub body: serde_json::Value,
    pub legal_basis: String,
    /// Check against the original text before relying on it.
    pub needs_legal_review: bool,
}

/// Legal rules with effective dates.
#[async_trait::async_trait]
pub trait ComplianceRules: Send + Sync {
    /// Version of rule `key` in force at `at` for the tenant's jurisdiction (default `vn`).
    async fn effective(
        &self,
        tenant: TenantId,
        key: &str,
        at: Timestamp,
    ) -> Result<Option<Rule>, Error>;
}

/// Event emitted when an identification profile changes status: the payload has `profile_id`, `subject_kind`,
/// `subject_id`, `status`. Calling modules (e.g. `plans` with `Subject::Reference`) listen to continue.
pub const KYC_VERIFIED: &str = "compliance.kyc_verified";
pub const KYC_REJECTED: &str = "compliance.kyc_rejected";
/// Event emitted when a verified identification reaches its end date (`expires_at`; the daily job) or is revoked by
/// a reviewer: the payload has `profile_id`, `subject_kind`, `subject_id` (and `reason_code` when revoked). The gate
/// already refuses an expired profile at the moment it is asked; modules that let a subject keep doing something
/// (e.g. `catalog`: goods of a marketplace seller on sale) listen to stop it.
pub const KYC_EXPIRED: &str = "compliance.kyc_expired";
pub const KYC_REVOKED: &str = "compliance.kyc_revoked";

/// Event emitted when a consent is withdrawn: the payload has `consent_id`, `purpose`, `scope`, `channel`.
/// Modules that send marketing messages (e.g. `notifications`) listen to cancel pending messages.
pub const CONSENT_WITHDRAWN: &str = "compliance.consent_withdrawn";

/// Event emitted when a user finishes verifying a phone number (payload `user_id`, no number) or removes a
/// verified number.
pub const PHONE_VERIFIED: &str = "compliance.phone_verified";
pub const PHONE_REMOVED: &str = "compliance.phone_removed";

/// Kinds of mandatory pages that can be attached to a website (the Ministry of Industry and Trade checklist,
/// page templates of `content`). Same keys as the page templates
/// `modules/content/templates/legal/<key>.<language>.json`.
pub const PAGE_KINDS: [&str; 11] = [
    "owner_info",
    "privacy_policy",
    "return_policy",
    "shipping_policy",
    "payment_policy",
    "inspection_policy",
    "complaint_policy",
    "terms_of_use",
    "general_conditions",
    "platform_rules",
    "seller_agreement",
];

/// The website for which a checklist "fact" is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebsiteRef {
    pub id: Uuid,
    /// Node of the website (root node = the whole tenant).
    pub org: OrgId,
    pub domain: String,
    /// `sales` or `platform`.
    pub kind: String,
}

/// A checklist condition answered by another module (e.g. step 46: "the product page shows the seller's
/// information"). `true` = satisfied.
#[async_trait::async_trait]
pub trait Fact: Send + Sync {
    async fn holds(&self, tenant: TenantId, website: &WebsiteRef) -> Result<bool, Error>;
}

/// Where modules registered after `compliance` contribute "facts" to the Ministry of Industry and Trade
/// checklist (the `check.type = "fact"` entries of rule `moit.checklist.*`). Nobody has registered that key →
/// the entry reports `unknown`.
pub trait ComplianceFacts: Send + Sync {
    /// Registers `fact` for key `key` (e.g. `marketplace.seller_info_on_product`); a key that already exists → error.
    fn register(&self, key: &'static str, fact: std::sync::Arc<dyn Fact>) -> Result<(), Error>;
}

/// Legal profile (the website owner) for other modules to use as template variables — e.g. `content` fills
/// the mandatory page templates. Returns nothing beyond information that must be public on the website
/// anyway.
#[async_trait::async_trait]
pub trait LegalProfiles: Send + Sync {
    /// Template variables (`owner.*`, `contact.*`) of the profile that applies to node `org` — the node's own
    /// profile, or that of the nearest ancestor node that has one (`None`: the whole tenant). No profile at
    /// all → `None`.
    async fn template_variables(
        &self,
        tenant: &TenantContext,
        org: Option<OrgId>,
    ) -> Result<Option<serde_json::Value>, Error>;
}

/// Event emitted when the Ministry of Industry and Trade notification / registration status of a website
/// changes (payload `website_id`, `domain`, `kind`, `status`, `previous_status`).
pub const WEBSITE_STATUS_CHANGED: &str = "compliance.website_status_changed";

/// Consumer content moderation rule — moved to the kernel (`autoxide_kernel::moderation::ModerationPolicy`) so
/// modules that hide reviews ask it without depending on `compliance`; re-exported here, same trait.
pub use autoxide_kernel::moderation::ModerationPolicy;

/// A consumer complaint forwarded from a support ticket (step 53): `compliance` creates a
/// `consumer_complaint` case with a statutory deadline (Law 19/2023/QH15), whose target is the ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplaintFromTicket {
    /// Source ticket, `support.ticket:<id>`: calling again with the same value returns the existing case, no second case is created.
    pub source_ref: String,
    /// The node being complained about (the ticket's store or stall).
    pub org_id: Uuid,
    /// The buyer's account, if any.
    pub reporter_user_id: Option<Uuid>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    /// Complaint content (10 to 5000 characters).
    pub claim: String,
}

/// A case that received a complaint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaseRef {
    pub id: Uuid,
    /// `false`: the ticket was forwarded earlier; this is the existing case.
    pub created: bool,
}

/// Receives complaints from other modules. Call OUTSIDE the caller's write transaction.
#[async_trait::async_trait]
pub trait ComplaintIntake: Send + Sync {
    async fn open(
        &self,
        tenant: &TenantContext,
        complaint: ComplaintFromTicket,
    ) -> Result<CaseRef, Error>;
}
