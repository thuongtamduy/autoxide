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
    PHONE_REMOVED, PhoneRemoved, "compliance.phone_removed", "A user removed their verified phone number (`user_id`).";
    POLICY_UPDATED, PolicyUpdated, "compliance.policy_updated", "A compliance policy of the tenant was set (`key`).";
    RULES_IMPORTED, RulesImported, "compliance.rules_imported", "Legal rules were imported (`jurisdiction`, `rules`).";
}
