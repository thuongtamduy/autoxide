//! Event module `compliance` phát. Payload chỉ ID và trạng thái — không email, số điện thoại, số giấy
//! tờ.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

macro_rules! events {
    ($($constant:ident, $kind:ident, $name:literal, $description:literal;)*) => {
        $(
            pub(crate) const $constant: EventInfo = EventInfo::new($name, $description);

            #[derive(Debug, Serialize)]
            pub(crate) struct $kind(pub Value);

            impl Event for $kind {
                const NAME: &'static str = $name;
            }
        )*

        pub(crate) fn all() -> Vec<EventInfo> {
            vec![$($constant),*]
        }
    };
}

events! {
    KYC_SUBMITTED, KycSubmitted, "compliance.kyc_submitted", "An identity verification started or documents were sent (`profile_id`, `subject_kind`, `subject_id`, `status`).";
    KYC_VERIFIED, KycVerified, "compliance.kyc_verified", "An identity was verified (`profile_id`, `subject_kind`, `subject_id`, `status`, `expires_at`).";
    KYC_REJECTED, KycRejected, "compliance.kyc_rejected", "An identity verification was refused (`profile_id`, `subject_kind`, `subject_id`, `status`, `reason_code`).";
    KYC_INFO_REQUESTED, KycInfoRequested, "compliance.kyc_info_requested", "More information was asked for a verification (`profile_id`, `subject_kind`, `subject_id`).";
    KYC_EXPIRED, KycExpired, "compliance.kyc_expired", "A verified identity reached its end date (`profile_id`, `subject_kind`, `subject_id`).";
    KYC_REVOKED, KycRevoked, "compliance.kyc_revoked", "A verified identity was revoked (`profile_id`, `subject_kind`, `subject_id`, `reason_code`).";
    DOCUMENT_VERSION_PUBLISHED, DocumentVersionPublished, "compliance.document_version_published", "A new version of a legal document was published (`document_id`, `kind`, `version`, `requires_reacceptance`).";
    DOCUMENT_ACCEPTED, DocumentAccepted, "compliance.document_accepted", "A legal document version was accepted (`document_id`, `version`, `subject_kind`, `subject_id`, `channel`).";
    CONSENT_GRANTED, ConsentGranted, "compliance.consent_granted", "A consent was given (`consent_id`, `purpose`, `scope`, `channel`).";
    CONSENT_WITHDRAWN, ConsentWithdrawn, "compliance.consent_withdrawn", "A consent was withdrawn (`consent_id`, `purpose`, `scope`, `channel`).";
    PHONE_VERIFIED, PhoneVerified, "compliance.phone_verified", "A user confirmed a phone number with a one-time code (`user_id`).";
    PHONE_UNLOCKED, PhoneUnlocked, "compliance.phone_unlocked", "An administrator unlocked sign-in codes for a phone number locked after too many wrong codes (`phone_key`: the tenant's key of the number, not the number).";
    PHONE_REMOVED, PhoneRemoved, "compliance.phone_removed", "A user removed their verified phone number (`user_id`).";
    POLICY_UPDATED, PolicyUpdated, "compliance.policy_updated", "A compliance policy of the tenant was set (`key`).";
    LEGAL_PROFILE_UPDATED, LegalProfileUpdated, "compliance.legal_profile_updated", "A legal (owner) profile was created or changed (`profile_id`, `org_id`).";
    WEBSITE_CREATED, WebsiteCreated, "compliance.website_created", "A website was added for the Ministry of Industry and Trade checklist (`website_id`, `org_id`, `domain`, `kind`).";
    WEBSITE_UPDATED, WebsiteUpdated, "compliance.website_updated", "A website or its linked mandatory pages changed (`website_id`, `domain`, `kind`).";
    WEBSITE_STATUS_CHANGED, WebsiteStatusChanged, "compliance.website_status_changed", "The notification / registration status of a website with the Ministry of Industry and Trade changed (`website_id`, `domain`, `kind`, `status`, `previous_status`).";
    PURPOSE_UPDATED, PurposeUpdated, "compliance.purpose_updated", "A processing purpose of the tenant was added or changed (`key`, `legal_basis`, `active`).";
    RESIDENCY_CHANGED, ResidencyChanged, "compliance.residency_changed", "Where the tenant must keep its data changed (`data_residency`, `previous`).";
    INCIDENT_OPENED, IncidentOpened, "compliance.incident_opened", "A personal data breach was recorded (`incident_id`, `severity`, `status`, `report_due_at`).";
    INCIDENT_UPDATED, IncidentUpdated, "compliance.incident_updated", "A personal data breach changed (`incident_id`, `severity`, `status`, `previous_status`).";
    INCIDENT_DEADLINE_NEAR, IncidentDeadlineNear, "compliance.incident_deadline_near", "A personal data breach is not reported yet and a reminder milestone passed (`incident_id`, `hours_since_detection`, `report_due_at`).";
    SUBJECT_REQUEST_RECEIVED, SubjectRequestReceived, "compliance.subject_request_received", "A request of a data subject received through another channel was recorded (`request_id`, `kind`, `channel`, `due_at`).";
    SUBJECT_REQUEST_UPDATED, SubjectRequestUpdated, "compliance.subject_request_updated", "A request of a data subject changed status (`request_id`, `kind`, `status`, `previous_status`).";
    TRACKING_CHANGED, TrackingChanged, "compliance.tracking_changed", "A tracking tag or the cookie banner of a website changed (`org_id`, `tag_id`, `provider`, `status`, `action`).";
    CASE_OPENED, CaseOpened, "compliance.case_opened", "A report of an infringement or a consumer complaint was received (`case_id`, `kind`, `status`, `target`, `reporter_kind`, `restricted`, `respondent_org_id`, `respondent_user_ids`).";
    CASE_UPDATED, CaseUpdated, "compliance.case_updated", "A case changed: note, answer of the reported party, restriction, closing (`case_id`, `kind`, `status`, `previous_status`, `target`, `restricted`).";
    CASE_DECIDED, CaseDecided, "compliance.case_decided", "A case was decided (`case_id`, `kind`, `status`, `decision`, `target`, `restricted`).";
    CASE_OVERDUE, CaseOverdue, "compliance.case_overdue", "An open case passed its deadline to acknowledge (`step` = `ack`) or to decide (`decide`) (`case_id`, `kind`, `status`, `due_at`).";
    TARGET_RESTRICTED, TargetRestricted, "compliance.target_restricted", "The target of a case must be hidden by the module that owns it (`case_id`, `target`, `case_kind`; contract `autoxide_kernel::moderation`).";
    TARGET_RESTORED, TargetRestored, "compliance.target_restored", "The target of a case may be shown again (`case_id`, `target`, `case_kind`; contract `autoxide_kernel::moderation`).";
    DISCLOSURE_RECORDED, DisclosureRecorded, "compliance.disclosure_recorded", "Information about a seller or user was given to a competent authority (`disclosure_id`, `case_id`, `subject`, `fields` — names only, no values).";
    RULES_IMPORTED, RulesImported, "compliance.rules_imported", "Legal rules were imported (`jurisdiction`, `rules`).";
}
