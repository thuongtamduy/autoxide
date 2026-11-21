//! Tool MCP của dữ liệu cá nhân và mã theo dõi (D-360, D-361): cùng service, cùng permission với REST.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::mcp::{NoInput, PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http_privacy::{
    DraftResponse, IncidentEntryResponse, IncidentResponse, SubjectRequestResponse,
    describe_incident, describe_request,
};
use crate::http_tracking::TagResponse;
use crate::incidents::NewIncident;
use crate::permissions::{INCIDENTS_MANAGE, PRIVACY_MANAGE, PRIVACY_READ, TRACKING_MANAGE};
use crate::service::ComplianceService;
use crate::subject_requests::NewRequest;

/// Records with a deadline.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeadlineInput {
    /// Only records with this status.
    status: Option<String>,
    /// Only records past their deadline and not done.
    #[serde(default)]
    overdue: bool,
    /// Maximum number of items, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// A personal data breach to record.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct IncidentInput {
    title: String,
    #[serde(default)]
    description: String,
    /// `low`, `medium`, `high` or `critical`.
    severity: String,
    /// When it was detected (the 72-hour deadline counts from here); defaults to now.
    detected_at: Option<Timestamp>,
    #[serde(default)]
    data_categories: Vec<String>,
    affected_count: Option<i64>,
}

/// A line for the log of a breach.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryInput {
    incident_id: Uuid,
    body: String,
}

/// Which draft.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftInput {
    incident_id: Uuid,
    /// `authority` or `subjects`.
    audience: String,
    /// `vi` or `en` (default).
    locale: Option<String>,
}

/// A request of a data subject received through another channel.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    /// `access`, `erasure`, `rectification`, `objection`, `restriction` or `withdrawal`.
    kind: String,
    /// `email`, `post`, `phone`, `authority` or `other`.
    channel: String,
    /// A user id, an email address or a phone number.
    subject: String,
    /// When it was received; defaults to now.
    received_at: Option<Timestamp>,
    notes: Option<String>,
}

/// Which tags.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TagsInput {
    /// Only the tags of this unit (website).
    org_id: Option<Uuid>,
    /// Maximum number of items, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

pub(crate) fn all(service: &Arc<ComplianceService>) -> Vec<Tool> {
    let (purposes, retention, register, incidents, open) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    let (entry, draft, requests, record, tags) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "purposes",
            "List the tenant's purposes for processing personal data with their legal basis \
             (Vietnam, Personal Data Protection Law 91/2025/QH15).",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let service = purposes.clone();
                async move { service.list_purposes(&tenant, &principal).await }
            },
        )
        .requires(PRIVACY_READ)
        .read_only(),
        Tool::new(
            "retention",
            "How long each kind of personal data is kept, compared with the limits of the legal \
             rule `privacy.retention_limits` (`ok`, `exceeds_limit`, `forever_not_allowed`).",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let service = retention.clone();
                async move { service.retention(&tenant, &principal).await }
            },
        )
        .requires(PRIVACY_READ)
        .read_only(),
        Tool::new(
            "processing_register",
            "The register of processing activities: purposes, legal bases, kinds of data, \
             recipients, retention and where the data is kept.",
            move |tenant: TenantContext, principal: Principal, _: NoInput| {
                let service = register.clone();
                async move { service.processing_register(&tenant, &principal).await }
            },
        )
        .requires(PRIVACY_READ)
        .read_only(),
        Tool::new(
            "incidents",
            "List personal data breaches with their 72-hour reporting deadline (`overdue` for those \
             late).",
            move |tenant: TenantContext, principal: Principal, input: DeadlineInput| {
                let service = incidents.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let found = service
                        .list_incidents(&tenant, &principal, input.status.as_deref(), input.overdue, &page)
                        .await?;
                    Ok::<Page<IncidentResponse>, Error>(found.map(|row| describe_incident(row, None)))
                }
            },
        )
        .requires(INCIDENTS_MANAGE),
        Tool::new(
            "open_incident",
            "Record a personal data breach. The report to the authority is due 72 hours after \
             detection; reminders go to the `incident.alert_emails` policy.",
            move |tenant: TenantContext, principal: Principal, input: IncidentInput| {
                let service = open.clone();
                async move {
                    let row = service
                        .open_incident(
                            &tenant,
                            &principal,
                            NewIncident {
                                title: input.title,
                                description: input.description,
                                severity: input.severity,
                                detected_at: input.detected_at,
                                data_categories: input.data_categories,
                                affected_count: input.affected_count,
                            },
                        )
                        .await?;
                    Ok::<IncidentResponse, Error>(describe_incident(row, None))
                }
            },
        )
        .requires(INCIDENTS_MANAGE),
        Tool::new(
            "add_incident_entry",
            "Add a line (what was found, what was done) to the log of a personal data breach.",
            move |tenant: TenantContext, principal: Principal, input: EntryInput| {
                let service = entry.clone();
                async move {
                    let row =
                        service.add_incident_entry(&tenant, &principal, input.incident_id, &input.body).await?;
                    Ok::<IncidentEntryResponse, Error>(row.into())
                }
            },
        )
        .requires(INCIDENTS_MANAGE),
        Tool::new(
            "incident_draft",
            "A draft notification of a personal data breach to the authority (`authority`) or to \
             the people affected (`subjects`). A draft only: a person completes, reviews and sends it.",
            move |tenant: TenantContext, principal: Principal, input: DraftInput| {
                let service = draft.clone();
                async move {
                    let language =
                        input.locale.as_deref().and_then(Language::from_tag).unwrap_or(Language::En);
                    let (subject, text) = service
                        .incident_draft(&tenant, &principal, input.incident_id, &input.audience, language)
                        .await?;
                    Ok::<DraftResponse, Error>(DraftResponse {
                        audience: input.audience,
                        locale: language.tag().to_owned(),
                        subject,
                        text,
                        disclaimer: crate::privacy::DISCLAIMER.to_owned(),
                    })
                }
            },
        )
        .requires(INCIDENTS_MANAGE),
        Tool::new(
            "subject_requests",
            "List the requests of data subjects received by email, post, phone or from an \
             authority, with their due date (`overdue` for those late).",
            move |tenant: TenantContext, principal: Principal, input: DeadlineInput| {
                let service = requests.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let found = service
                        .list_subject_requests(&tenant, &principal, input.status.as_deref(), input.overdue, &page)
                        .await?;
                    Ok::<Page<SubjectRequestResponse>, Error>(found.map(describe_request))
                }
            },
        )
        .requires(PRIVACY_MANAGE)
        .read_only(),
        Tool::new(
            "record_subject_request",
            "Record a request of a data subject received through another channel; the due date \
             comes from the legal rule `privacy.subject_requests`. Fulfilling it (export / erase) \
             is done through REST by a person.",
            move |tenant: TenantContext, principal: Principal, input: RequestInput| {
                let service = record.clone();
                async move {
                    let row = service
                        .record_subject_request(
                            &tenant,
                            &principal,
                            NewRequest {
                                kind: input.kind,
                                channel: input.channel,
                                subject: input.subject,
                                received_at: input.received_at,
                                notes: input.notes,
                            },
                        )
                        .await?;
                    Ok::<SubjectRequestResponse, Error>(describe_request(row))
                }
            },
        )
        .requires(PRIVACY_MANAGE),
        Tool::new(
            "tracking_tags",
            "List the advertising and analytics tracking tags of the websites in your scope (GA4, \
             GTM, Google Ads, Meta, TikTok, Zalo); they load only after visitor consent.",
            move |tenant: TenantContext, principal: Principal, input: TagsInput| {
                let service = tags.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let found = service.list_tags(&tenant, &principal, input.org_id, &page).await?;
                    Ok::<Page<TagResponse>, Error>(found.map(TagResponse::from))
                }
            },
        )
        .requires(TRACKING_MANAGE),
    ]
}
