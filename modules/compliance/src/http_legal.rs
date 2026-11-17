//! Route REST của văn bản pháp lý, chấp nhận, đồng ý, quy tắc và chính sách (dưới
//! `/api/v1/compliance`).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{Consent, ConsentEvidence, ConsentKey, ConsentSubject, Rule, Subject};
use crate::documents::{NewAcceptance, NewVersion};
use crate::entity::{acceptance, document, policy, version};
use crate::service::{ComplianceService, describe_rule};

pub(crate) fn routes(service: Arc<ComplianceService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_documents, create_document))
        .routes(routes!(get_document))
        .routes(routes!(publish_version))
        .routes(routes!(list_acceptances))
        .routes(routes!(acceptance_stats))
        .routes(routes!(pending_acceptances))
        .routes(routes!(accept))
        .routes(routes!(public_document))
        .routes(routes!(my_consents, set_my_consent))
        .routes(routes!(list_rules, import_rules))
        .routes(routes!(list_policies))
        .routes(routes!(set_policy))
        .with_state(service)
}

// ---- Văn bản -------------------------------------------------------------------------------------

/// A version of a legal document (never changes once published).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceDocumentVersion)]
pub(crate) struct VersionResponse {
    id: Uuid,
    document_id: Uuid,
    version: i32,
    locale: String,
    title: String,
    body: String,
    /// SHA-256 of title and body: proof of the exact text accepted.
    sha256: String,
    effective_from: DateTime<Utc>,
    /// People must accept this version again before going on.
    requires_reacceptance: bool,
    published_at: DateTime<Utc>,
}

impl From<version::Model> for VersionResponse {
    fn from(row: version::Model) -> Self {
        VersionResponse {
            id: row.id,
            document_id: row.document_id,
            version: row.version,
            locale: row.locale,
            title: row.title,
            body: row.body,
            sha256: row.sha256,
            effective_from: row.effective_from,
            requires_reacceptance: row.requires_reacceptance,
            published_at: row.published_at,
        }
    }
}

/// A legal document of the tenant.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceDocument)]
pub(crate) struct DocumentResponse {
    id: Uuid,
    /// `terms_of_use`, `privacy_policy`, `general_conditions`, `platform_rules`, `seller_agreement`
    /// or `developer_agreement`.
    kind: String,
    /// `user`, `buyer` or `seller`.
    audience: String,
    title: String,
    current_version: Option<i32>,
    /// Every version and translation, newest first (only on `GET /documents/{id}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    versions: Option<Vec<VersionResponse>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub(crate) fn describe_document(
    row: document::Model,
    versions: Option<Vec<version::Model>>,
) -> DocumentResponse {
    DocumentResponse {
        id: row.id,
        kind: row.kind,
        audience: row.audience,
        title: row.title,
        current_version: row.current_version,
        versions: versions.map(|rows| rows.into_iter().map(VersionResponse::from).collect()),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// A legal document to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceDocumentRequest)]
pub(crate) struct DocumentRequest {
    #[schema(example = "terms_of_use")]
    kind: String,
    /// `user` (default), `buyer` or `seller`.
    audience: Option<String>,
    title: String,
}

/// A version to publish.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceVersionRequest)]
pub(crate) struct VersionRequest {
    #[schema(example = "vi")]
    locale: String,
    title: String,
    /// Full text (Markdown or HTML as the interface shows it). Have a legal professional review it.
    body: String,
    /// When it takes effect; default now.
    effective_from: Option<DateTime<Utc>>,
    /// Everyone must accept it again (the first version always must).
    #[serde(default)]
    requires_reacceptance: bool,
    /// To add a translation of a version already published, its number.
    version: Option<i32>,
}

/// List the legal documents of the tenant.
///
/// Requires `compliance.documents.read`.
#[utoipa::path(
    get,
    path = "/documents",
    extensions(("x-permissions" = json!(["compliance.documents.read"]))),
    operation_id = "list_compliance_documents",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The documents.", body = Data<Vec<DocumentResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.documents.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_documents(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<DocumentResponse>>, Error> {
    let rows = service.list_documents(&tenant, &principal).await?;
    Ok(Data { data: rows.into_iter().map(|row| describe_document(row, None)).collect() })
}

/// Create a legal document.
///
/// Requires `compliance.documents.manage`. One document per kind; publish versions of it.
#[utoipa::path(
    post,
    path = "/documents",
    extensions(("x-permissions" = json!(["compliance.documents.manage"]))),
    operation_id = "create_compliance_document",
    security(("bearer" = [])),
    request_body = DocumentRequest,
    responses(
        (status = 201, description = "The document.", body = Data<DocumentResponse>, headers(("Location" = String, description = "Path of the document."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.documents.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "A document of this kind exists; code `compliance.document_exists`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn create_document(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<DocumentRequest>,
) -> Result<Created<DocumentResponse>, Error> {
    let audience = input.audience.unwrap_or_else(|| "user".to_owned());
    let row =
        service.create_document(&tenant, &principal, input.kind, audience, input.title).await?;
    Ok(Created::new(format!("/compliance/documents/{}", row.id), describe_document(row, None)))
}

/// Get a legal document with its versions.
///
/// Requires `compliance.documents.read`.
#[utoipa::path(
    get,
    path = "/documents/{id}",
    extensions(("x-permissions" = json!(["compliance.documents.read"]))),
    operation_id = "get_compliance_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document.")),
    responses(
        (status = 200, description = "The document.", body = Data<DocumentResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.documents.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document; code `compliance.document_not_found`.", body = Problem),
    )
)]
async fn get_document(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<DocumentResponse>, Error> {
    let row = service.document_for(&tenant, &principal, id).await?;
    let versions = service.versions_of(&tenant, row.id).await?;
    Ok(Data { data: describe_document(row, Some(versions)) })
}

/// Publish a new version (or a translation) of a legal document.
///
/// Requires `compliance.documents.manage`. A published version never changes. Have a legal
/// professional review the text first.
#[utoipa::path(
    post,
    path = "/documents/{id}/versions",
    extensions(("x-permissions" = json!(["compliance.documents.manage"]))),
    operation_id = "publish_compliance_document_version",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document.")),
    request_body = VersionRequest,
    responses(
        (status = 201, description = "The version.", body = Data<VersionResponse>, headers(("Location" = String, description = "Path of the document."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.documents.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document; code `compliance.document_not_found`.", body = Problem),
        (status = 409, description = "This version already has a text in this language; code `compliance.version_exists`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn publish_version(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<VersionRequest>,
) -> Result<Created<VersionResponse>, Error> {
    let version = NewVersion {
        locale: input.locale,
        title: input.title,
        body: input.body,
        effective_from: input.effective_from,
        requires_reacceptance: input.requires_reacceptance,
        version: input.version,
    };
    let row = service.publish_version(&tenant, &principal, id, version).await?;
    Ok(Created::new(format!("/compliance/documents/{id}"), row.into()))
}

/// Proof that a version was accepted.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceAcceptance)]
pub(crate) struct AcceptanceResponse {
    id: Uuid,
    document_id: Uuid,
    version: i32,
    version_id: Uuid,
    subject_kind: String,
    subject_id: String,
    channel: String,
    method: String,
    /// SHA-256 of the text accepted.
    sha256: String,
    accepted_at: DateTime<Utc>,
}

impl From<acceptance::Model> for AcceptanceResponse {
    fn from(row: acceptance::Model) -> Self {
        AcceptanceResponse {
            id: row.id,
            document_id: row.document_id,
            version: row.version,
            version_id: row.version_id,
            subject_kind: row.subject_kind,
            subject_id: row.subject_id,
            channel: row.channel,
            method: row.method,
            sha256: row.sha256,
            accepted_at: row.accepted_at,
        }
    }
}

/// List who accepted a document.
///
/// Requires `compliance.documents.read`.
#[utoipa::path(
    get,
    path = "/documents/{id}/acceptances",
    extensions(("x-permissions" = json!(["compliance.documents.read"]))),
    operation_id = "list_compliance_acceptances",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document."), PageQuery),
    responses(
        (status = 200, description = "One page of acceptances.", body = Page<AcceptanceResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.documents.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document; code `compliance.document_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_acceptances(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<AcceptanceResponse>, Error> {
    let page =
        service.list_acceptances(&tenant, &principal, id, &PageRequest::try_from(page)?).await?;
    Ok(page.map(AcceptanceResponse::from))
}

/// How many accepted one version.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceAcceptanceCount)]
pub(crate) struct AcceptanceCount {
    version: i32,
    accepted: u64,
}

/// Count acceptances per version of a document.
///
/// Requires `compliance.documents.read`.
#[utoipa::path(
    get,
    path = "/documents/{id}/acceptance-stats",
    extensions(("x-permissions" = json!(["compliance.documents.read"]))),
    operation_id = "get_compliance_acceptance_stats",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document.")),
    responses(
        (status = 200, description = "Counts per version.", body = Data<Vec<AcceptanceCount>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.documents.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document; code `compliance.document_not_found`.", body = Problem),
    )
)]
async fn acceptance_stats(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<AcceptanceCount>>, Error> {
    let stats = service.acceptance_stats(&tenant, &principal, id).await?;
    Ok(Data {
        data: stats
            .into_iter()
            .map(|(version, accepted)| AcceptanceCount { version, accepted })
            .collect(),
    })
}

/// A document you still have to accept.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CompliancePendingAcceptance)]
pub(crate) struct PendingAcceptance {
    document_id: Uuid,
    kind: String,
    title: String,
    version: VersionResponse,
}

/// List the legal documents you must accept (again).
///
/// Any signed-in user.
#[utoipa::path(
    get,
    path = "/me/pending-acceptances",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_pending_acceptances",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Documents to accept.", body = Data<Vec<PendingAcceptance>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn pending_acceptances(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<PendingAcceptance>>, Error> {
    let rows = service.pending_acceptances(&tenant, &Subject::User(principal.user_id())).await?;
    Ok(Data {
        data: rows
            .into_iter()
            .map(|(doc, current)| PendingAcceptance {
                document_id: doc.id,
                kind: doc.kind,
                title: doc.title,
                version: current.into(),
            })
            .collect(),
    })
}

/// An acceptance to record.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceAcceptRequest)]
pub(crate) struct AcceptRequest {
    /// The version shown to the person.
    version_id: Uuid,
    /// `web` (default), `mobile_app`, `api`, `mcp` or `in_store`.
    channel: Option<String>,
    /// `checkbox` (default), `click_wrap` or `signature`.
    method: Option<String>,
}

/// Accept a version of a legal document.
///
/// Any signed-in user. Accepting again changes nothing.
#[utoipa::path(
    post,
    path = "/me/acceptances",
    extensions(("x-permissions" = json!([]))),
    operation_id = "accept_compliance_document",
    security(("bearer" = [])),
    request_body = AcceptRequest,
    responses(
        (status = 200, description = "The proof of acceptance.", body = Data<AcceptanceResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such version; code `compliance.version_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn accept(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    client: ClientInfo,
    ValidJson(input): ValidJson<AcceptRequest>,
) -> Result<Data<AcceptanceResponse>, Error> {
    let acceptance = NewAcceptance {
        version_id: input.version_id,
        channel: input.channel.unwrap_or_else(|| "web".to_owned()),
        method: input.method.unwrap_or_else(|| "checkbox".to_owned()),
        ip_fingerprint: client
            .ip
            .map(|ip| autoxide_kernel::secret::sha256(&format!("acceptance-ip|{ip}"))),
        user_agent: client.user_agent.clone(),
    };
    let row = service
        .accept(&tenant, Actor::from(&principal), &Subject::User(principal.user_id()), acceptance)
        .await?;
    Ok(Data { data: row.into() })
}

/// Which language.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct LocaleQuery {
    /// Language code, e.g. `vi`; default the first text of the current version.
    locale: Option<String>,
}

/// Get the current text of a legal document.
///
/// No sign-in (the tenant comes from the domain). Shows the version in force, with its SHA-256.
#[utoipa::path(
    get,
    path = "/public/documents/{kind}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_compliance_document",
    params(("kind" = String, Path, description = "Kind of document, e.g. `terms_of_use`."), LocaleQuery),
    responses(
        (status = 200, description = "The current version.", body = Data<VersionResponse>),
        (status = 404, description = "The tenant has no such document in force; code `compliance.document_not_found`.", body = Problem),
    )
)]
async fn public_document(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    Path(kind): Path<String>,
    Query(query): Query<LocaleQuery>,
) -> Result<Data<VersionResponse>, Error> {
    let (_, current) = service.public_document(&tenant, &kind, query.locale.as_deref()).await?;
    Ok(Data { data: current.into() })
}

// ---- Đồng ý --------------------------------------------------------------------------------------

/// A consent to give or withdraw.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceConsentRequest)]
pub(crate) struct ConsentRequest {
    #[schema(example = "marketing")]
    purpose: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    channel: String,
    /// `true` to give consent, `false` to withdraw it.
    granted: bool,
    /// The sentence shown to you (required to give consent).
    text: Option<String>,
    /// Version of that sentence.
    text_version: Option<String>,
}

/// List your consents.
///
/// Any signed-in user.
#[utoipa::path(
    get,
    path = "/me/consents",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_consents",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Your consents.", body = Data<Vec<Consent>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn my_consents(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<Consent>>, Error> {
    Ok(Data { data: service.my_consents(&tenant, &principal).await? })
}

/// Give or withdraw a consent.
///
/// Any signed-in user. Withdrawing is as easy as giving, and stops sending at once.
#[utoipa::path(
    put,
    path = "/me/consents",
    extensions(("x-permissions" = json!([]))),
    operation_id = "set_my_consent",
    security(("bearer" = [])),
    request_body = ConsentRequest,
    responses(
        (status = 200, description = "The consent after the change.", body = Data<Consent>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn set_my_consent(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    client: ClientInfo,
    ValidJson(input): ValidJson<ConsentRequest>,
) -> Result<Data<Consent>, Error> {
    let subject = ConsentSubject::User(principal.user_id());
    let key = ConsentKey::new(input.purpose, input.scope, input.channel);
    let source_ref = format!("me:{}:{}", principal.user_id(), Uuid::now_v7());
    let actor = Actor::from(&principal);
    let consent = if input.granted {
        let (Some(text), Some(text_version)) = (input.text, input.text_version) else {
            return Err(Error::validation(vec![FieldError::new(
                "text",
                "required",
                "send the sentence shown and its version to give consent",
            )]));
        };
        let evidence = ConsentEvidence {
            text,
            text_version,
            source: "me".to_owned(),
            ip_fingerprint: client
                .ip
                .map(|ip| autoxide_kernel::secret::sha256(&format!("consent-ip|{ip}"))),
        };
        service.grant_consent(&tenant, actor, &subject, &key, &evidence, &source_ref).await?
    } else {
        service.withdraw_consent(&tenant, actor, &subject, &key, "me", &source_ref).await?
    };
    Ok(Data { data: consent })
}

// ---- Quy tắc, chính sách -------------------------------------------------------------------------

/// List the legal rules (every version).
///
/// Requires `compliance.rules.read`.
#[utoipa::path(
    get,
    path = "/rules",
    extensions(("x-permissions" = json!(["compliance.rules.read"]))),
    operation_id = "list_compliance_rules",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The rules, newest version first per key.", body = Data<Vec<Rule>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.rules.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_rules(
    State(service): State<Arc<ComplianceService>>,
    principal: Principal,
) -> Result<Data<Vec<Rule>>, Error> {
    Ok(Data { data: service.list_rules(&principal).await?.iter().map(describe_rule).collect() })
}

/// Rules to import.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceRulesRequest)]
pub(crate) struct RulesRequest {
    /// A rules pack in the format of `modules/compliance/rules/vn.json`.
    #[schema(value_type = Object)]
    pack: serde_json::Value,
}

/// How many rules were new.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceRulesImported)]
pub(crate) struct RulesImported {
    imported: u64,
}

/// Import legal rules.
///
/// Platform staff with `compliance.rules.manage`. Rules are only added: a version already imported
/// stays as it is.
#[utoipa::path(
    post,
    path = "/rules",
    extensions(("x-permissions" = json!(["compliance.rules.manage"]))),
    operation_id = "import_compliance_rules",
    security(("bearer" = [])),
    request_body = RulesRequest,
    responses(
        (status = 200, description = "How many were new.", body = Data<RulesImported>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `compliance.rules.manage` (`permission_denied`).", body = Problem),
        (status = 422, description = "The pack is invalid (`compliance.rules_invalid`); every problem in `errors`.", body = Problem),
    )
)]
async fn import_rules(
    State(service): State<Arc<ComplianceService>>,
    principal: Principal,
    ValidJson(input): ValidJson<RulesRequest>,
) -> Result<Data<RulesImported>, Error> {
    let imported = service.import_rules_manual(&principal, &input.pack.to_string()).await?;
    Ok(Data { data: RulesImported { imported } })
}

/// A compliance policy of the tenant.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CompliancePolicy)]
pub(crate) struct PolicyResponse {
    key: String,
    /// Any JSON value, checked by the policy (see `PUT /policies/{key}`).
    value: serde_json::Value,
    updated_at: DateTime<Utc>,
}

impl From<policy::Model> for PolicyResponse {
    fn from(row: policy::Model) -> Self {
        PolicyResponse {
            key: row.key,
            value: serde_json::from_str(&row.value).unwrap_or_default(),
            updated_at: row.updated_at,
        }
    }
}

/// A policy value.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CompliancePolicyRequest)]
pub(crate) struct PolicyRequest {
    /// Any JSON value, checked by the policy (see `PUT /policies/{key}`).
    value: serde_json::Value,
}

/// List the compliance policies set by the tenant.
///
/// Requires `compliance.policies.manage`.
#[utoipa::path(
    get,
    path = "/policies",
    extensions(("x-permissions" = json!(["compliance.policies.manage"]))),
    operation_id = "list_compliance_policies",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The policies set (others use their defaults).", body = Data<Vec<PolicyResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.policies.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_policies(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<PolicyResponse>>, Error> {
    Ok(Data {
        data: service
            .list_policies(&tenant, &principal)
            .await?
            .into_iter()
            .map(PolicyResponse::from)
            .collect(),
    })
}

/// Set a compliance policy.
///
/// Requires `compliance.policies.manage`. Keys: `jurisdiction`, `posting.comments`,
/// `posting.reviews`, `posting.livestream` (`none`, `phone` or `identity`),
/// `posting.max_accounts_per_phone`, `complaints.contact`, `privacy.contact`, `incident.alert_emails`.
#[utoipa::path(
    put,
    path = "/policies/{key}",
    extensions(("x-permissions" = json!(["compliance.policies.manage"]))),
    operation_id = "set_compliance_policy",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Policy key.")),
    request_body = PolicyRequest,
    responses(
        (status = 200, description = "The policy.", body = Data<PolicyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.policies.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such policy; code `compliance.policy_unknown`.", body = Problem),
        (status = 422, description = "The value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn set_policy(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    ValidJson(input): ValidJson<PolicyRequest>,
) -> Result<Data<PolicyResponse>, Error> {
    Ok(Data { data: service.set_policy(&tenant, &principal, &key, input.value).await?.into() })
}
