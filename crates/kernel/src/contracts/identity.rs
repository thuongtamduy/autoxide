//! Identity verification (KYC) through a provider (step 37, D-186): neutral traits that let the `compliance`
//! module use VNeID (through an authorized provider), eKYC or business registration lookup without depending on
//! any specific provider. A provider's adapter is its own module that publishes
//! `context.provide::<dyn IdentityVerifier>(...)`. Tests use `FakeIdentityVerifier` from the testkit.
//!
//! Minimal storage (Personal Data Protection Law 91/2025): `compliance` stores only the result, the reference
//! code, the verified name, and a hash plus the last 4 digits of the document number; it stores no face images
//! or biometric data.
use crate::time::{Date, Timestamp};
use uuid::Uuid;

use crate::error::{Error, ErrorKind};
use crate::tenancy::TenantId;

/// Kind of party being verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartyType {
    Individual,
    HouseholdBusiness,
    Organization,
}

impl PartyType {
    pub fn as_str(self) -> &'static str {
        match self {
            PartyType::Individual => "individual",
            PartyType::HouseholdBusiness => "household_business",
            PartyType::Organization => "organization",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "individual" => Some(PartyType::Individual),
            "household_business" => Some(PartyType::HouseholdBusiness),
            "organization" => Some(PartyType::Organization),
            _ => None,
        }
    }
}

/// Verification method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerificationMethod {
    /// VNeID electronic identity account (through an authorized provider).
    Vneid,
    /// eKYC: photo of the document plus a face image, matched by the provider.
    Ekyc,
    /// Business registration lookup.
    BusinessRegistry,
    /// A reviewer inspects an uploaded document.
    Manual,
}

impl VerificationMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            VerificationMethod::Vneid => "vneid",
            VerificationMethod::Ekyc => "ekyc",
            VerificationMethod::BusinessRegistry => "business_registry",
            VerificationMethod::Manual => "manual",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "vneid" => Some(VerificationMethod::Vneid),
            "ekyc" => Some(VerificationMethod::Ekyc),
            "business_registry" => Some(VerificationMethod::BusinessRegistry),
            "manual" => Some(VerificationMethod::Manual),
            _ => None,
        }
    }
}

/// A verification attempt to start.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerificationRequest {
    pub tenant: TenantId,
    /// ID of the attempt in this system (the provider sends it back in the callback).
    pub attempt_id: Uuid,
    pub party: PartyType,
    pub method: VerificationMethod,
    pub return_url: Option<String>,
    pub locale: String,
}

impl VerificationRequest {
    pub fn new(
        tenant: TenantId,
        attempt_id: Uuid,
        party: PartyType,
        method: VerificationMethod,
    ) -> Self {
        VerificationRequest {
            tenant,
            attempt_id,
            party,
            method,
            return_url: None,
            locale: "vi".to_owned(),
        }
    }

    #[must_use]
    pub fn return_url(mut self, url: Option<String>) -> Self {
        self.return_url = url;
        self
    }

    #[must_use]
    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = locale.into();
        self
    }
}

/// How the user continues the verification attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationSession {
    /// Opens the provider's page.
    Redirect { provider_reference: String, url: String, expires_at: Timestamp },
    /// A mobile app opens the SDK with a token.
    Sdk { provider_reference: String, token: String, expires_at: Timestamp },
    /// Nothing for the user to do (e.g. business registration lookup); the result arrives later.
    Pending { provider_reference: String },
}

impl VerificationSession {
    /// Verification attempt code at the provider.
    pub fn provider_reference(&self) -> &str {
        match self {
            VerificationSession::Redirect { provider_reference, .. }
            | VerificationSession::Sdk { provider_reference, .. }
            | VerificationSession::Pending { provider_reference } => provider_reference,
        }
    }
}

/// Verification outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutcomeStatus {
    Verified,
    Rejected { reason_code: String },
    Pending,
}

/// Result returned by the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerificationOutcome {
    pub provider_reference: String,
    pub status: OutcomeStatus,
    pub verified_name: Option<String>,
    /// Document type (`cccd`, `passport`, `business_registration`, ...).
    pub document_kind: Option<String>,
    /// Document number: `compliance` stores only a hash and the last 4 digits, never the full number.
    pub document_number: Option<String>,
    /// Assurance level, e.g. `vneid_level_2`.
    pub assurance: Option<String>,
    pub expires_at: Option<Timestamp>,
    /// Date of birth of a verified individual.
    pub birth_date: Option<Date>,
    /// Address (an individual's) or head office (an organization's, a household business's).
    pub address: Option<String>,
    /// Legal representative of an organization or a household business.
    pub representative_name: Option<String>,
    /// Country that issued the document, ISO 3166-1 alpha-2 (`VN`, `US`, ...).
    pub document_country: Option<String>,
    /// Last day the document is valid (a passport must stay valid long enough; see the `compliance` rule).
    pub document_expires_on: Option<Date>,
}

impl VerificationOutcome {
    pub fn new(provider_reference: impl Into<String>, status: OutcomeStatus) -> Self {
        VerificationOutcome {
            provider_reference: provider_reference.into(),
            status,
            verified_name: None,
            document_kind: None,
            document_number: None,
            assurance: None,
            expires_at: None,
            birth_date: None,
            address: None,
            representative_name: None,
            document_country: None,
            document_expires_on: None,
        }
    }

    #[must_use]
    pub fn verified_name(mut self, name: impl Into<String>) -> Self {
        self.verified_name = Some(name.into());
        self
    }

    #[must_use]
    pub fn document(mut self, kind: impl Into<String>, number: impl Into<String>) -> Self {
        self.document_kind = Some(kind.into());
        self.document_number = Some(number.into());
        self
    }

    #[must_use]
    pub fn assurance(mut self, assurance: impl Into<String>) -> Self {
        self.assurance = Some(assurance.into());
        self
    }

    /// Date of birth of a verified individual.
    #[must_use]
    pub fn birth_date(mut self, date: Date) -> Self {
        self.birth_date = Some(date);
        self
    }

    /// Address of an individual, head office of an organization or a household business.
    #[must_use]
    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    /// Legal representative of an organization or a household business.
    #[must_use]
    pub fn representative(mut self, name: impl Into<String>) -> Self {
        self.representative_name = Some(name.into());
        self
    }

    /// Issuing country (ISO 3166-1 alpha-2) and last valid day of the document.
    #[must_use]
    pub fn document_validity(mut self, country: impl Into<String>, expires_on: Date) -> Self {
        self.document_country = Some(country.into());
        self.document_expires_on = Some(expires_on);
        self
    }
}

/// An identity verification provider.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait IdentityVerifier: Send + Sync {
    /// Stable name (`vnpt_ekyc`, `fake`, ...): part of the callback path.
    fn provider(&self) -> &'static str;

    fn methods(&self) -> Vec<VerificationMethod>;

    async fn start(&self, request: VerificationRequest) -> Result<VerificationSession, Error>;

    async fn fetch(&self, provider_reference: &str) -> Result<VerificationOutcome, Error>;

    /// Verifies the callback signature over the raw body; a wrong signature yields [`callback_invalid`].
    async fn verify_callback(
        &self,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<VerificationOutcome, Error>;
}

/// `identity_verification_disabled` (503): no provider is configured.
pub fn verification_disabled() -> Error {
    Error::unavailable(
        "identity_verification_disabled",
        "No identity verification provider is set up on this server.",
    )
}

/// `identity_callback_invalid` (401): the callback signature is wrong.
pub fn callback_invalid() -> Error {
    Error::new(
        ErrorKind::Unauthenticated,
        "identity_callback_invalid",
        "The signature of the identity provider callback is not valid.",
    )
}
