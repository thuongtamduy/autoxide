//! Permission của module `compliance`.

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const KYC_READ: Permission = Permission::new(
    "compliance.kyc.read",
    "See the identity verification (KYC) profiles of the units and people in your tenant.",
);

pub(crate) const KYC_SUBMIT: Permission = Permission::new(
    "compliance.kyc.submit",
    "Open identity verification profiles for units in your scope and start verifications.",
)
.scoped();

pub(crate) const KYC_REVIEW: Permission = Permission::new(
    "compliance.kyc.review",
    "Approve, reject, revoke identity verification profiles and ask for more information (signed-in \
     person only, not API keys).",
)
.with_risk(Risk::SensitiveData);

pub(crate) const DOCUMENTS_READ: Permission = Permission::new(
    "compliance.documents.read",
    "See legal documents, their versions and who accepted them.",
);

pub(crate) const DOCUMENTS_MANAGE: Permission = Permission::new(
    "compliance.documents.manage",
    "Create legal documents and publish new versions.",
)
.with_risk(Risk::Write);

pub(crate) const PRIVACY_READ: Permission = Permission::new(
    "compliance.privacy.read",
    "See consents recorded in the tenant, the processing purposes, the retention of personal data, \
     the processing register and where the tenant's data is kept.",
);

pub(crate) const PRIVACY_MANAGE: Permission = Permission::new(
    "compliance.privacy.manage",
    "Set the processing purposes of the tenant and handle the requests of data subjects received \
     through other channels (export or erase their data).",
)
.with_risk(Risk::SensitiveData);

pub(crate) const INCIDENTS_MANAGE: Permission = Permission::new(
    "compliance.incidents.manage",
    "Record personal data breaches, follow their 72-hour reporting deadline and draft the \
     notifications.",
)
.with_risk(Risk::Write);

pub(crate) const TRACKING_MANAGE: Permission = Permission::new(
    "compliance.tracking.manage",
    "Set the advertising and analytics tags of websites in your scope and their cookie consent \
     banner, and see consent statistics.",
)
.scoped()
.with_risk(Risk::Write);

pub(crate) const POLICIES_MANAGE: Permission = Permission::new(
    "compliance.policies.manage",
    "Set the compliance policies of the tenant (posting rules, contacts…).",
)
.with_risk(Risk::Write);

pub(crate) const RULES_READ: Permission = Permission::new(
    "compliance.rules.read",
    "See the legal rules in force (data with effective dates).",
);

pub(crate) const RULES_MANAGE: Permission = Permission::new(
    "compliance.rules.manage",
    "Platform staff: import new versions of the legal rules for every tenant.",
)
.with_risk(Risk::Write);

pub(crate) const LEGAL_READ: Permission = Permission::new(
    "compliance.legal.read",
    "See the legal (owner) profiles and websites, their checklist and dossier for the Ministry of \
     Industry and Trade.",
)
.scoped();

pub(crate) const LEGAL_MANAGE: Permission = Permission::new(
    "compliance.legal.manage",
    "Set the legal (owner) profiles and websites of units in your scope, link their mandatory \
     pages and record their status with the Ministry of Industry and Trade.",
)
.scoped()
.with_risk(Risk::Write);

pub(crate) fn all() -> Vec<Permission> {
    vec![
        KYC_READ,
        KYC_SUBMIT,
        KYC_REVIEW,
        DOCUMENTS_READ,
        DOCUMENTS_MANAGE,
        PRIVACY_READ,
        PRIVACY_MANAGE,
        INCIDENTS_MANAGE,
        TRACKING_MANAGE,
        POLICIES_MANAGE,
        RULES_READ,
        RULES_MANAGE,
        LEGAL_READ,
        LEGAL_MANAGE,
    ]
}
