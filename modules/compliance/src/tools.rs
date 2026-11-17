//! Tool MCP của `compliance`: cùng service, cùng permission với REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{NoInput, PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::{KycStatus, Subject};
use crate::documents::NewVersion;
use crate::http_kyc::ProfileResponse;
use crate::http_legal::{DocumentResponse, VersionResponse, describe_document};
use crate::permissions::{DOCUMENTS_MANAGE, DOCUMENTS_READ, KYC_READ};
use crate::service::ComplianceService;

/// Which profiles.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfilesInput {
    /// `pending`, `in_review`, `info_requested`, `verified`, `rejected`, `expired` or `revoked`.
    status: Option<String>,
    /// Maximum number of profiles, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// A version to publish.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishInput {
    document_id: Uuid,
    /// e.g. `vi`.
    locale: String,
    title: String,
    /// Full text. Have a legal professional review it first.
    body: String,
    /// Everyone must accept it again.
    #[serde(default)]
    requires_reacceptance: bool,
    /// To add a translation of a version already published, its number.
    version: Option<i32>,
}

/// A document to publish (with its versions).
#[derive(Debug, serde::Serialize)]
pub(crate) struct Pending {
    document_id: Uuid,
    kind: String,
    title: String,
    version: VersionResponse,
}

pub(crate) fn all(service: &Arc<ComplianceService>) -> Vec<Tool> {
    let (status, profiles, documents, publish, pending) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "verification_status",
            "Your identity verification (KYC) status: `none`, `pending`, `in_review`, \
             `info_requested`, `verified`, `rejected`, `expired` or `revoked`.",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let service = status.clone();
                async move {
                    let found: KycStatus =
                        service.kyc_status(&tenant, &Subject::User(principal.user_id())).await?;
                    Ok::<_, Error>(found)
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "kyc_profiles",
            "List the identity verification profiles of the tenant (results only, never document \
             images or full numbers).",
            move |tenant: TenantContext, principal: Principal, input: ProfilesInput| {
                let service = profiles.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let found =
                        service.list_profiles(&tenant, &principal, input.status, &page).await?;
                    Ok::<Page<ProfileResponse>, Error>(found.map(ProfileResponse::from))
                }
            },
        )
        .requires(KYC_READ)
        .read_only(),
        Tool::new(
            "documents",
            "List the legal documents of the tenant (terms of use, privacy policy…) with their \
             current version number.",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let service = documents.clone();
                async move {
                    let rows = service.list_documents(&tenant, &principal).await?;
                    Ok::<Vec<DocumentResponse>, Error>(
                        rows.into_iter().map(|row| describe_document(row, None)).collect(),
                    )
                }
            },
        )
        .requires(DOCUMENTS_READ)
        .read_only(),
        Tool::new(
            "publish_document_version",
            "Publish a new version (or a translation) of a legal document. A published version never \
             changes. Draft texts with AI only as suggestions for a person to review.",
            move |tenant: TenantContext, principal: Principal, input: PublishInput| {
                let service = publish.clone();
                async move {
                    let version = NewVersion {
                        locale: input.locale,
                        title: input.title,
                        body: input.body,
                        effective_from: None,
                        requires_reacceptance: input.requires_reacceptance,
                        version: input.version,
                    };
                    let row =
                        service.publish_version(&tenant, &principal, input.document_id, version).await?;
                    Ok::<VersionResponse, Error>(row.into())
                }
            },
        )
        .requires(DOCUMENTS_MANAGE),
        Tool::new(
            "pending_acceptances",
            "The legal documents you must accept (again) before going on.",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let service = pending.clone();
                async move {
                    let rows = service
                        .pending_acceptances(&tenant, &Subject::User(principal.user_id()))
                        .await?;
                    Ok::<Vec<Pending>, Error>(
                        rows.into_iter()
                            .map(|(doc, current)| Pending {
                                document_id: doc.id,
                                kind: doc.kind,
                                title: doc.title,
                                version: current.into(),
                            })
                            .collect(),
                    )
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
    ]
}
