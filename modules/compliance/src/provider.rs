//! Trait module `compliance` công bố: `KycGate`, `AcceptanceGate`, `ConsentLedger`, `ComplianceRules`,
//! `PostingGate`, `AccountVerification`; và bộ hỏi `PostingVerifier` của kernel (D-202).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::identity::PartyType;
use autoxide_kernel::tenancy::TenantId;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use autoxide_kernel::access::{PostingVerifier, Principal};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::TenantContext;

use crate::api::{
    AcceptanceGate, AccountVerification, Activity, ComplianceRules, Consent, ConsentEvidence,
    ConsentKey, ConsentLedger, ConsentSubject, KycGate, KycSession, KycStatus, PostingGate,
    PostingSurface, Rule, Subject, VerificationLevel,
};
use crate::service::{ComplianceService, describe_rule};

pub(crate) struct Provider(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl KycGate for Provider {
    async fn require_verified(
        &self,
        tenant: TenantId,
        subject: Subject,
        activity: Activity,
    ) -> Result<(), Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.require_verified(&tenant, &subject, activity).await
    }

    async fn status(&self, tenant: TenantId, subject: Subject) -> Result<KycStatus, Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.kyc_status(&tenant, &subject).await
    }

    async fn start(
        &self,
        tenant: TenantId,
        subject: Subject,
        party: PartyType,
        return_url: Option<String>,
    ) -> Result<KycSession, Error> {
        let tenant = self.0.tenant(tenant).await?;
        let profile = self.0.open_profile(&tenant, &subject, party).await?;
        self.0.start_session(&tenant, Actor::system(), profile.id, None, return_url).await
    }
}

#[async_trait::async_trait]
impl AcceptanceGate for Provider {
    async fn require_accepted(
        &self,
        tenant: TenantId,
        subject: Subject,
        kind: &str,
    ) -> Result<(), Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.require_accepted(&tenant, &subject, kind).await
    }

    async fn accept_current(
        &self,
        tenant: TenantId,
        subject: Subject,
        kind: &str,
        channel: &str,
    ) -> Result<Option<Uuid>, Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.accept_current(&tenant, &subject, kind, channel).await
    }
}

#[async_trait::async_trait]
impl ConsentLedger for Provider {
    async fn grant(
        &self,
        tenant: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        evidence: ConsentEvidence,
        source_ref: &str,
    ) -> Result<Consent, Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.grant_consent(&tenant, Actor::system(), &subject, &key, &evidence, source_ref).await
    }

    async fn withdraw(
        &self,
        tenant: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        source: &str,
        source_ref: &str,
    ) -> Result<Consent, Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.withdraw_consent(&tenant, Actor::system(), &subject, &key, source, source_ref).await
    }

    async fn is_granted(
        &self,
        tenant: TenantId,
        subject: &ConsentSubject,
        key: &ConsentKey,
    ) -> Result<bool, Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.is_granted(&tenant, subject, key).await
    }

    async fn granted_many(
        &self,
        tenant: TenantId,
        subjects: &[ConsentSubject],
        key: &ConsentKey,
    ) -> Result<Vec<bool>, Error> {
        let tenant = self.0.tenant(tenant).await?;
        let mut answers = Vec::with_capacity(subjects.len());
        for subject in subjects {
            answers.push(self.0.is_granted(&tenant, subject, key).await?);
        }
        Ok(answers)
    }

    async fn get(&self, tenant: TenantId, consent_id: Uuid) -> Result<Option<Consent>, Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.consent(&tenant, consent_id).await
    }
}

#[async_trait::async_trait]
impl ComplianceRules for Provider {
    async fn effective(
        &self,
        tenant: TenantId,
        key: &str,
        at: DateTime<Utc>,
    ) -> Result<Option<Rule>, Error> {
        let tenant = self.0.tenant(tenant).await?;
        Ok(self.0.rule_for(&tenant, key, at).await?.as_ref().map(describe_rule))
    }
}

#[async_trait::async_trait]
impl PostingGate for Provider {
    async fn check(
        &self,
        tenant: TenantId,
        principal: Option<&Principal>,
        surface: PostingSurface,
    ) -> Result<(), Error> {
        let tenant = self.0.tenant(tenant).await?;
        self.0.check_posting(&tenant, principal, surface).await
    }
}

#[async_trait::async_trait]
impl AccountVerification for Provider {
    async fn level(&self, tenant: TenantId, user_id: Uuid) -> Result<VerificationLevel, Error> {
        let tenant = self.0.tenant(tenant).await?;
        Ok(self.0.verification(&tenant, user_id).await?.level)
    }

    async fn verified_phone(
        &self,
        tenant: TenantId,
        user_id: Uuid,
    ) -> Result<Option<PhoneNumber>, Error> {
        let tenant = self.0.tenant(tenant).await?;
        Ok(self.0.verification(&tenant, user_id).await?.phone)
    }
}

/// Bộ hỏi của kernel cho `comments`: "đã xác thực" nghĩa là đạt mức `posting.comments` của khách, và
/// ít nhất là số điện thoại — loại đích nằm trong `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES` vẫn cần xác
/// thực khi khách để chính sách là `none`.
#[async_trait::async_trait]
impl PostingVerifier for Provider {
    async fn is_verified(&self, tenant: &TenantContext, user_id: Uuid) -> Result<bool, Error> {
        let required = self
            .0
            .posting_level(tenant, PostingSurface::Comments)
            .await?
            .max(VerificationLevel::Phone);
        Ok(self.0.verification(tenant, user_id).await?.level >= required)
    }

    async fn required_everywhere(&self, tenant: &TenantContext) -> Result<bool, Error> {
        Ok(self.0.posting_level(tenant, PostingSurface::Comments).await? != VerificationLevel::None)
    }
}

#[async_trait::async_trait]
impl crate::api::LegalProfiles for Provider {
    async fn template_variables(
        &self,
        tenant: &TenantContext,
        org: Option<autoxide_kernel::tenancy::OrgId>,
    ) -> Result<Option<serde_json::Value>, Error> {
        self.0.template_variables(tenant, org).await
    }
}

impl crate::api::ComplianceFacts for Provider {
    fn register(&self, key: &'static str, fact: Arc<dyn crate::api::Fact>) -> Result<(), Error> {
        self.0.facts.register(key, fact)
    }
}
