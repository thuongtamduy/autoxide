//! Route REST của mẫu khách tự soạn (D-206, D-366) dưới `/api/v1/notifications`: schema biến của loại,
//! bản nháp, xuất bản, quay lại, xem trước, gửi thử, thương hiệu của mail. Permission được kiểm trong
//! service TRƯỚC mọi bước tra cứu hay kiểm đầu vào.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use crate::api::Channel;
use crate::branding::BrandingInput;
use crate::custom::{Content, Preview, Source, sample_of, schema_of};
use crate::entity::{branding, template_draft, template_version};
use crate::service::{NotificationsService, kind_unknown};

pub(crate) fn routes(service: Arc<NotificationsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(kind_schema))
        .routes(routes!(get_draft, put_draft, delete_draft))
        .routes(routes!(list_versions))
        .routes(routes!(publish))
        .routes(routes!(rollback))
        .routes(routes!(unpublish))
        .routes(routes!(preview))
        .routes(routes!(test_send))
        .routes(routes!(get_branding, put_branding))
        .with_state(service)
}

// ---- Schema biến ---------------------------------------------------------------------------------

/// A variable a template of this kind may use.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationTemplateVariable)]
pub(crate) struct VariableResponse {
    /// Dotted path, used as `{{ path }}`; a parent allows every field below it.
    path: String,
    /// The template must use it (on the main part of the channel).
    required: bool,
    description: String,
}

/// The variables of a kind and sample data for previews.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationKindSchema)]
pub(crate) struct SchemaResponse {
    kind: String,
    variables: Vec<VariableResponse>,
    /// Sample values used by previews and test sends.
    #[schema(value_type = Object)]
    sample: Value,
}

pub(crate) fn describe_schema(
    service: &NotificationsService,
    key: &str,
) -> Result<SchemaResponse, Error> {
    let kind = service.kinds.get(key).ok_or_else(|| kind_unknown(key))?;
    Ok(SchemaResponse {
        kind: kind.key().to_owned(),
        variables: schema_of(&kind)
            .variables
            .into_iter()
            .map(|variable| VariableResponse {
                path: variable.path.to_owned(),
                required: variable.required,
                description: variable.description.to_owned(),
            })
            .collect(),
        sample: sample_of(&kind),
    })
}

/// Show the template variables of a notification kind.
///
/// Any signed-in user. Templates use the syntax of the template engine: `{{ name }}`, `{% if %}`,
/// `{% for %}`, filters `money`, `number`, `date`.
#[utoipa::path(
    get,
    path = "/kinds/{kind}/schema",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_notification_kind_schema",
    security(("bearer" = [])),
    params(("kind" = String, Path, description = "Key of the kind, e.g. `orders.order_shipped`.")),
    responses(
        (status = 200, description = "The variables and sample data.", body = Data<SchemaResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "Unknown kind; code `notifications.kind_unknown`.", body = Problem),
    )
)]
async fn kind_schema(
    State(service): State<Arc<NotificationsService>>,
    _principal: Principal,
    Path(kind): Path<String>,
) -> Result<Data<SchemaResponse>, Error> {
    Ok(Data { data: describe_schema(&service, &kind)? })
}

// ---- Bản nháp ------------------------------------------------------------------------------------

/// The draft of a template (not in use until published).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationTemplateDraft)]
pub(crate) struct DraftResponse {
    kind: String,
    channel: String,
    locale: String,
    subject: String,
    body_text: String,
    body_html: Option<String>,
    short: String,
    zns_template_id: Option<String>,
    updated_at: Timestamp,
}

impl From<template_draft::Model> for DraftResponse {
    fn from(row: template_draft::Model) -> Self {
        DraftResponse {
            kind: row.kind,
            channel: row.channel,
            locale: row.locale,
            subject: row.subject,
            body_text: row.body_text,
            body_html: row.body_html,
            short: row.short,
            zns_template_id: row.zns_template_id,
            updated_at: row.updated_at,
        }
    }
}

/// A template written with the template engine (`{{ name }}`, `{% if %}`, `{% for %}`). Only the
/// variables of `GET /kinds/{kind}/schema` may be used; required ones must be used. Values put into
/// `body_html` are always HTML-escaped.
#[derive(Debug, Clone, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationTemplateDraftRequest)]
pub(crate) struct DraftRequest {
    /// Email subject, push and inbox title. Required on `email`, `in_app`, `web_push`, `mobile_push`.
    #[serde(default)]
    #[schema(example = "Order {{ code }} is on its way")]
    pub subject: String,
    /// Plain text: the text part of the email and the inbox body. Required on `email` and `in_app`.
    #[serde(default)]
    pub body_text: String,
    /// HTML of the email body (`email` only), placed in the branded mail layout; omitted → made
    /// from `body_text`.
    pub body_html: Option<String>,
    /// Short text for SMS, ZNS and push. Required on `sms`, `zns`, `web_push`, `mobile_push`.
    #[serde(default)]
    pub short: String,
    /// Approved Zalo ZNS template id (channel `zns`).
    pub zns_template_id: Option<String>,
}

impl From<DraftRequest> for Content {
    fn from(input: DraftRequest) -> Self {
        Content {
            subject: input.subject,
            body_text: input.body_text,
            body_html: input.body_html,
            short: input.short,
            zns_template_id: input.zns_template_id,
        }
    }
}

/// Show the draft of a template.
///
/// Requires `notifications.templates.read` or `notifications.templates.manage`.
#[utoipa::path(
    get,
    path = "/templates/{kind}/{channel}/{locale}/draft",
    extensions(("x-permissions" = json!(["notifications.templates.read"]))),
    operation_id = "get_notification_template_draft",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "One channel of the kind, e.g. `email`."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    responses(
        (status = 200, description = "The draft.", body = Data<DraftResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No draft; code `notifications.template_draft_not_found`.", body = Problem),
        (status = 422, description = "Unknown kind, channel or language; code `notifications.kind_unknown` or `validation_failed`.", body = Problem),
    )
)]
async fn get_draft(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
) -> Result<Data<DraftResponse>, Error> {
    let row = service.get_draft(&tenant, &principal, (&kind, &channel, &locale)).await?;
    Ok(Data { data: row.into() })
}

/// Save the draft of a template.
///
/// Requires `notifications.templates.manage`. Checked on save: syntax, only variables of the kind,
/// required variables present — every problem in `errors` at once, with line and column. Advertising
/// always gets the label and the unsubscribe footer added; a template cannot remove them.
#[utoipa::path(
    put,
    path = "/templates/{kind}/{channel}/{locale}/draft",
    extensions(("x-permissions" = json!(["notifications.templates.manage"]))),
    operation_id = "set_notification_template_draft",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "One channel of the kind."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    request_body = DraftRequest,
    responses(
        (status = 200, description = "The saved draft.", body = Data<DraftResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Unknown kind (`notifications.kind_unknown`), a template too big (`template.limit_exceeded`), or invalid fields (`validation_failed`; each problem has code `template.syntax_error`, `template.unknown_variable`, `template.required_variable_missing` or a length rule).", body = Problem),
    )
)]
async fn put_draft(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
    ValidJson(input): ValidJson<DraftRequest>,
) -> Result<Data<DraftResponse>, Error> {
    let row =
        service.put_draft(&tenant, &principal, (&kind, &channel, &locale), input.into()).await?;
    Ok(Data { data: row.into() })
}

/// Throw the draft of a template away.
///
/// Requires `notifications.templates.manage`. The published version, if any, stays in use.
#[utoipa::path(
    delete,
    path = "/templates/{kind}/{channel}/{locale}/draft",
    extensions(("x-permissions" = json!(["notifications.templates.manage"]))),
    operation_id = "delete_notification_template_draft",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "Channel."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No draft; code `notifications.template_draft_not_found`.", body = Problem),
        (status = 422, description = "Unknown kind, channel or language.", body = Problem),
    )
)]
async fn delete_draft(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
) -> Result<StatusCode, Error> {
    service.delete_draft(&tenant, &principal, (&kind, &channel, &locale)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Phiên bản -----------------------------------------------------------------------------------

/// A published version of a template; it never changes.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationTemplateVersion)]
pub(crate) struct VersionResponse {
    kind: String,
    channel: String,
    locale: String,
    version: i32,
    /// This version is the one in use.
    live: bool,
    subject: String,
    body_text: String,
    body_html: Option<String>,
    short: String,
    zns_template_id: Option<String>,
    published_at: Timestamp,
}

pub(crate) fn describe_version(row: template_version::Model, live: Option<i32>) -> VersionResponse {
    VersionResponse {
        live: live == Some(row.version),
        kind: row.kind,
        channel: row.channel,
        locale: row.locale,
        version: row.version,
        subject: row.subject,
        body_text: row.body_text,
        body_html: row.body_html,
        short: row.short,
        zns_template_id: row.zns_template_id,
        published_at: row.published_at,
    }
}

/// List the published versions of a template, newest first.
///
/// Requires `notifications.templates.read` or `notifications.templates.manage`. The last 20 versions
/// are kept; `live` marks the one in use. Newest first unless `order=asc`; paged with `limit` /
/// `cursor`.
#[utoipa::path(
    get,
    path = "/templates/{kind}/{channel}/{locale}/versions",
    extensions(("x-permissions" = json!(["notifications.templates.read"]))),
    operation_id = "list_notification_template_versions",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "Channel."),
        ("locale" = String, Path, description = "`en` or `vi`."),
        PageQuery,
    ),
    responses(
        (status = 200, description = "One page of the versions (maybe none), newest first.", body = Page<VersionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Unknown kind, channel or language, or a paging parameter is invalid.", body = Problem),
    )
)]
async fn list_versions(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
    Query(page): Query<PageQuery>,
) -> Result<Page<VersionResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    let target = (kind.as_str(), channel.as_str(), locale.as_str());
    let found = service.list_versions(&tenant, &principal, target, &page).await?;
    let live = found.live;
    Ok(found.items.map(|row| describe_version(row, live)))
}

/// Publish the draft of a template.
///
/// Requires `notifications.templates.manage`. The draft becomes a new version (it never changes
/// afterwards) and is used from the next notification on; the draft is removed.
#[utoipa::path(
    post,
    path = "/templates/{kind}/{channel}/{locale}/publish",
    extensions(("x-permissions" = json!(["notifications.templates.manage"]))),
    operation_id = "publish_notification_template",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "Channel."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    responses(
        (status = 200, description = "The new version, now in use.", body = Data<VersionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No draft; code `notifications.template_draft_not_found`.", body = Problem),
        (status = 422, description = "Unknown kind, channel or language, or the draft no longer fits the variables of the kind.", body = Problem),
    )
)]
async fn publish(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
) -> Result<Data<VersionResponse>, Error> {
    let row = service.publish_template(&tenant, &principal, (&kind, &channel, &locale)).await?;
    let version = row.version;
    Ok(Data { data: describe_version(row, Some(version)) })
}

/// Which version to use again.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationTemplateRollbackRequest)]
pub(crate) struct RollbackRequest {
    /// A version listed by `GET …/versions`.
    #[schema(example = 1)]
    version: i32,
}

/// Use an older published version of a template again.
///
/// Requires `notifications.templates.manage`. No new version is made; the draft is kept.
#[utoipa::path(
    post,
    path = "/templates/{kind}/{channel}/{locale}/rollback",
    extensions(("x-permissions" = json!(["notifications.templates.manage"]))),
    operation_id = "rollback_notification_template",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "Channel."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    request_body = RollbackRequest,
    responses(
        (status = 200, description = "The version now in use.", body = Data<VersionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such version; code `notifications.template_version_not_found`.", body = Problem),
        (status = 422, description = "Unknown kind, channel or language.", body = Problem),
    )
)]
async fn rollback(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
    ValidJson(input): ValidJson<RollbackRequest>,
) -> Result<Data<VersionResponse>, Error> {
    let row = service
        .rollback_template(&tenant, &principal, (&kind, &channel, &locale), input.version)
        .await?;
    let version = row.version;
    Ok(Data { data: describe_version(row, Some(version)) })
}

/// Stop using the published template.
///
/// Requires `notifications.templates.manage`. The replaced text (`PUT /templates/…`) or the default
/// text applies again; the versions are kept for a rollback.
#[utoipa::path(
    delete,
    path = "/templates/{kind}/{channel}/{locale}/published",
    extensions(("x-permissions" = json!(["notifications.templates.manage"]))),
    operation_id = "unpublish_notification_template",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "Channel."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    responses(
        (status = 204, description = "No longer in use."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "Nothing published in use; code `notifications.template_not_published`.", body = Problem),
        (status = 422, description = "Unknown kind, channel or language.", body = Problem),
    )
)]
async fn unpublish(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
) -> Result<StatusCode, Error> {
    service.unpublish_template(&tenant, &principal, (&kind, &channel, &locale)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Xem trước, gửi thử --------------------------------------------------------------------------

/// Which version: `draft` (default), `published` (the one in use) or `version` (with `version`).
pub(crate) fn parse_source(source: Option<&str>, version: Option<i32>) -> Result<Source, Error> {
    match (source.unwrap_or("draft"), version) {
        ("draft", None) => Ok(Source::Draft),
        ("published", None) => Ok(Source::Published),
        ("version", Some(version)) => Ok(Source::Version(version)),
        _ => Err(Error::validation(vec![FieldError::new(
            "source",
            "invalid",
            "must be `draft`, `published` or `version` (with `version`)",
        )])),
    }
}

/// What to preview.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationTemplatePreviewRequest)]
pub(crate) struct PreviewRequest {
    /// `draft` (default), `published` or `version`.
    pub source: Option<String>,
    /// With `source = version`.
    pub version: Option<i32>,
    /// Values to render with (only variables of the kind); omitted → the sample data of the kind.
    #[schema(value_type = Option<Object>)]
    pub data: Option<Value>,
}

/// A rendered template.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationTemplatePreview)]
pub(crate) struct PreviewResponse {
    subject: String,
    text: String,
    /// The complete HTML email in the branded layout (channel `email` only).
    html: Option<String>,
    short: String,
}

impl From<Preview> for PreviewResponse {
    fn from(preview: Preview) -> Self {
        PreviewResponse {
            subject: preview.subject,
            text: preview.text,
            html: preview.html,
            short: preview.short,
        }
    }
}

/// Preview a template.
///
/// Requires `notifications.templates.read` or `notifications.templates.manage`. Renders the draft, the
/// published version or an older version with the sample data of the kind or your `data`. Nothing is
/// sent.
#[utoipa::path(
    post,
    path = "/templates/{kind}/{channel}/{locale}/preview",
    extensions(("x-permissions" = json!(["notifications.templates.read"]))),
    operation_id = "preview_notification_template",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "Channel."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    request_body = PreviewRequest,
    responses(
        (status = 200, description = "The rendered template.", body = Data<PreviewResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such draft or version; code `notifications.template_draft_not_found`, `notifications.template_not_published` or `notifications.template_version_not_found`.", body = Problem),
        (status = 422, description = "Unknown kind, channel or language, a key of `data` the kind does not have (`template.unknown_variable`), or the template fails to render (`template.render_failed`, `template.limit_exceeded`).", body = Problem),
    )
)]
async fn preview(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
    ValidJson(input): ValidJson<PreviewRequest>,
) -> Result<Data<PreviewResponse>, Error> {
    // Quyền kiểm trong service trước khi đọc `source`.
    crate::custom::require_read(&principal)?;
    let source = parse_source(input.source.as_deref(), input.version)?;
    let rendered = service
        .preview_template(&tenant, &principal, (&kind, &channel, &locale), source, input.data)
        .await?;
    Ok(Data { data: rendered.into() })
}

/// Which version to send as a test.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationTemplateTestRequest)]
pub(crate) struct TestSendRequest {
    /// `draft` (default), `published` or `version`.
    source: Option<String>,
    /// With `source = version`.
    version: Option<i32>,
}

/// Where the test went.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationTemplateTestResult)]
pub(crate) struct TestSendResponse {
    /// `email` (your verified email) or `in_app` (your inbox).
    channel: Channel,
}

/// Send a template to yourself as a test.
///
/// Requires `notifications.templates.manage`. Rendered with the sample data of the kind and sent only
/// to the caller: on `email` to the caller's verified email, on `in_app` to the caller's inbox — never
/// to another address. At most 10 tests per hour.
#[utoipa::path(
    post,
    path = "/templates/{kind}/{channel}/{locale}/test-send",
    extensions(("x-permissions" = json!(["notifications.templates.manage"]))),
    operation_id = "test_send_notification_template",
    security(("bearer" = [])),
    params(
        ("kind" = String, Path, description = "Key of the kind."),
        ("channel" = String, Path, description = "`email` or `in_app`."),
        ("locale" = String, Path, description = "`en` or `vi`."),
    ),
    request_body = TestSendRequest,
    responses(
        (status = 200, description = "Sent.", body = Data<TestSendResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such draft or version.", body = Problem),
        (status = 422, description = "Unknown kind, channel or language; a channel other than `email` / `in_app` (`notifications.test_send_unsupported`); your email is not verified (`notifications.test_address_unverified`); or the template fails to render.", body = Problem),
        (status = 429, description = "Too many tests in the last hour; code `notifications.test_send_throttled`.", body = Problem),
    )
)]
async fn test_send(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((kind, channel, locale)): Path<(String, String, String)>,
    ValidJson(input): ValidJson<TestSendRequest>,
) -> Result<Data<TestSendResponse>, Error> {
    principal.require(&crate::permissions::TEMPLATES_MANAGE)?;
    let source = parse_source(input.source.as_deref(), input.version)?;
    let channel =
        service.test_send_template(&tenant, &principal, (&kind, &channel, &locale), source).await?;
    Ok(Data { data: TestSendResponse { channel } })
}

// ---- Thương hiệu ---------------------------------------------------------------------------------

/// The branding of the tenant's HTML emails.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = NotificationBranding)]
pub(crate) struct BrandingResponse {
    /// Shown at the top when there is no logo.
    brand_name: Option<String>,
    /// https URL of the logo image.
    logo_url: Option<String>,
    /// `#rrggbb`: top line and headings.
    primary_color: String,
    /// `#rrggbb`: page background.
    background_color: String,
    /// First footer line; the owner's legal name and address (legal profile) follow when set.
    footer_text: Option<String>,
}

impl From<branding::Model> for BrandingResponse {
    fn from(row: branding::Model) -> Self {
        BrandingResponse {
            brand_name: row.brand_name,
            logo_url: row.logo_url,
            primary_color: row.primary_color,
            background_color: row.background_color,
            footer_text: row.footer_text,
        }
    }
}

/// New branding (replaces the whole branding; omitted fields go back to the defaults).
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NotificationBrandingRequest)]
pub(crate) struct BrandingRequest {
    brand_name: Option<String>,
    /// An https URL.
    logo_url: Option<String>,
    /// `#rrggbb`; default `#1f6feb`.
    #[schema(example = "#1f6feb")]
    primary_color: Option<String>,
    /// `#rrggbb`; default `#f4f5f7`.
    background_color: Option<String>,
    footer_text: Option<String>,
}

/// Show the branding of the tenant's HTML emails.
///
/// Requires `notifications.templates.read`, `notifications.templates.manage` or
/// `notifications.branding.manage`.
#[utoipa::path(
    get,
    path = "/branding",
    extensions(("x-permissions" = json!(["notifications.templates.read"]))),
    operation_id = "get_notification_branding",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The branding (defaults when never set).", body = Data<BrandingResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.templates.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn get_branding(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<BrandingResponse>, Error> {
    Ok(Data { data: service.get_branding(&tenant, &principal).await?.into() })
}

/// Change the branding of the tenant's HTML emails.
///
/// Requires `notifications.branding.manage`. Every email of `notifications` uses it.
#[utoipa::path(
    put,
    path = "/branding",
    extensions(("x-permissions" = json!(["notifications.branding.manage"]))),
    operation_id = "set_notification_branding",
    security(("bearer" = [])),
    request_body = BrandingRequest,
    responses(
        (status = 200, description = "The new branding.", body = Data<BrandingResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `notifications.branding.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Invalid fields (every problem in `errors`).", body = Problem),
    )
)]
async fn put_branding(
    State(service): State<Arc<NotificationsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<BrandingRequest>,
) -> Result<Data<BrandingResponse>, Error> {
    let input = BrandingInput {
        brand_name: input.brand_name,
        logo_url: input.logo_url,
        primary_color: input.primary_color,
        background_color: input.background_color,
        footer_text: input.footer_text,
    };
    Ok(Data { data: service.put_branding(&tenant, &principal, input).await?.into() })
}
