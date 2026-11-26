//! Tool MCP của `compliance`: cùng service, cùng permission với REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{NoInput, PageInput, Tool};
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest};
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::{KycStatus, Subject};
use crate::documents::NewVersion;
use crate::http_kyc::ProfileResponse;
use crate::http_legal::{DocumentResponse, VersionResponse, describe_document};
use crate::http_moit::{LegalProfileResponse, WebsiteResponse, describe_website};
use crate::http_verification::VerificationResponse;
use crate::permissions::{DOCUMENTS_MANAGE, DOCUMENTS_READ, KYC_READ, LEGAL_MANAGE, LEGAL_READ};
use crate::service::ComplianceService;
use crate::websites::StatusChange;
use autoxide_kernel::version::IfMatch;

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
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// One page of a list in its natural order (kind, key…): the same as the `limit`, `cursor`, `order`
/// and `count` query parameters of the REST endpoint.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NaturalPageInput {
    /// Maximum number of items, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `asc` (the default, A→Z) or `desc`.
    order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

impl NaturalPageInput {
    /// Cùng kiểu tham số trang với REST (`NaturalOrderPageQuery`, mặc định `asc`).
    pub(crate) fn page(self) -> Result<PageRequest, Error> {
        PageRequest::try_from(
            NaturalOrderPageQuery::new(
                self.limit.map(|limit| limit.to_string()),
                self.cursor,
                self.order,
            )
            .with_count(self.count.unwrap_or(false)),
        )
    }
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

/// One page of a list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListInput {
    /// Maximum number of items, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// Which website.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct WebsiteInput {
    website_id: Uuid,
}

/// A status to record for a website.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct WebsiteStatusInput {
    website_id: Uuid,
    /// `not_submitted`, `submitted`, `needs_supplement`, `notified` (sales website),
    /// `registered` (marketplace), `rejected` or `revoked`.
    status: String,
    /// Number of the file at the ministry.
    reference: Option<String>,
    /// For `notified` / `registered`: the logo code the ministry gave, unchanged.
    snippet: Option<String>,
    /// The `ETag` read before (`"…"`), to avoid overwriting another change.
    version: Option<String>,
}

pub(crate) fn all(service: &Arc<ComplianceService>) -> Vec<Tool> {
    let mut tools = base(service);
    tools.extend(moit(service));
    tools.extend(crate::tools_privacy::all(service));
    tools.extend(crate::tools_cases::all(service));
    tools
}

/// Tool của hồ sơ Bộ Công Thương: cùng service, cùng permission với REST.
fn moit(service: &Arc<ComplianceService>) -> Vec<Tool> {
    let (profiles, websites, checklist, dossier, status) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "legal_profiles",
            "List the legal (owner) profiles of the tenant and its units: what Vietnamese law \
             requires a commerce website to show about its owner.",
            move |tenant: TenantContext, principal: Principal, input: ListInput| {
                let service = profiles.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let found = service.list_profiles_legal(&tenant, &principal, &page).await?;
                    Ok::<Page<LegalProfileResponse>, Error>(found.map(LegalProfileResponse::from))
                }
            },
        )
        .requires(LEGAL_READ)
        .read_only(),
        Tool::new(
            "websites",
            "List the websites of the tenant with their kind (`sales` → notify the Ministry of \
             Industry and Trade, `platform` → register) and status.",
            move |tenant: TenantContext, principal: Principal, input: ListInput| {
                let service = websites.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let found = service.list_websites(&tenant, &principal, &page).await?;
                    Ok::<Page<WebsiteResponse>, Error>(found.map(|row| describe_website(row, None)))
                }
            },
        )
        .requires(LEGAL_READ)
        .read_only(),
        Tool::new(
            "website_checklist",
            "What a website still lacks before it is notified to / registered with the Ministry of \
             Industry and Trade; each item is `met`, `missing`, `unknown` or `manual`, with what to do.",
            move |tenant: TenantContext, principal: Principal, input: WebsiteInput| {
                let service = checklist.clone();
                async move { service.checklist(&tenant, &principal, input.website_id).await }
            },
        )
        .requires(LEGAL_READ)
        .read_only(),
        Tool::new(
            "website_dossier",
            "The draft dossier of a website for the Ministry of Industry and Trade, filled from the \
             tenant's data (declaration; for a marketplace a draft project plan and regulations). A \
             person reviews it with a legal professional and submits it on online.gov.vn.",
            move |tenant: TenantContext, principal: Principal, input: WebsiteInput| {
                let service = dossier.clone();
                async move { service.dossier(&tenant, &principal, input.website_id).await }
            },
        )
        .requires(LEGAL_READ)
        .read_only(),
        Tool::new(
            "set_website_status",
            "Record what happened with a website's file at the Ministry of Industry and Trade \
             (submitted, notified, registered…). `notified` / `registered` need the logo code the \
             ministry gave.",
            move |tenant: TenantContext, principal: Principal, input: WebsiteStatusInput| {
                let service = status.clone();
                async move {
                    let expected = match input.version.as_deref() {
                        Some(version) => IfMatch::from_value(version).expected()?,
                        None => None,
                    };
                    let change = StatusChange {
                        status: input.status,
                        reference: input.reference,
                        snippet: input.snippet,
                        ..StatusChange::default()
                    };
                    let row = service
                        .set_website_status(&tenant, &principal, input.website_id, expected, change)
                        .await?;
                    Ok::<WebsiteResponse, Error>(describe_website(row, None))
                }
            },
        )
        .requires(LEGAL_MANAGE),
    ]
}

fn base(service: &Arc<ComplianceService>) -> Vec<Tool> {
    let (status, profiles, documents, publish, pending) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    let account = service.clone();
    vec![
        Tool::new(
            "account_verification",
            "Your account verification level for posting content (Vietnam, Decree 147/2024): \
             `none`, `phone` (mobile number confirmed by SMS code) or `identity` (identity \
             verified), with the verified number masked. Codes are sent and confirmed only through \
             REST (`POST /api/v1/compliance/me/phone-verifications`).",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let service = account.clone();
                async move {
                    let found = service.verification(&tenant, principal.user_id()).await?;
                    Ok::<VerificationResponse, Error>(found.into())
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
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
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
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
             current version number, by kind; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: NaturalPageInput| {
                let service = documents.clone();
                async move {
                    let page = input.page()?;
                    let rows = service.list_documents(&tenant, &principal, &page).await?;
                    Ok::<Page<DocumentResponse>, Error>(
                        rows.map(|row| describe_document(row, None)),
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
            "The legal documents you must accept (again) before going on, by kind; follow \
             `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: NaturalPageInput| {
                let service = pending.clone();
                async move {
                    let page = input.page()?;
                    let rows = service
                        .pending_acceptances(&tenant, &Subject::User(principal.user_id()), &page)
                        .await?;
                    Ok::<Page<Pending>, Error>(rows.map(|(doc, current)| Pending {
                        document_id: doc.id,
                        kind: doc.kind,
                        title: doc.title,
                        version: current.into(),
                    }))
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
    ]
}
