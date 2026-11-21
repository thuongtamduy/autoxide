//! Route REST của mẫu in, bản in, bản xuất kế toán và bản lưu trữ (dưới `/api/v1/invoices`).

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::{Disposition, ValidJson, content_disposition};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{KINDS, RenderFormat, Rendered};
use crate::entity::{archive, export, template};
use crate::exports::{ExportInput, FORMATS, PROFILES};
use crate::permissions::{ARCHIVE, EXPORT, READ};
use crate::print::{self, DEFAULT_TEMPLATE};
use crate::service::InvoicesService;
use crate::templates::{LOCALES, TemplateInput};

pub(crate) fn routes(service: Arc<InvoicesService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_templates, create_template))
        .routes(routes!(get_template, update_template, delete_template))
        .routes(routes!(preview_template))
        .routes(routes!(template_defaults))
        .routes(routes!(render_document))
        .routes(routes!(export_profiles))
        .routes(routes!(list_exports, create_export))
        .routes(routes!(get_export))
        .routes(routes!(retry_export))
        .routes(routes!(download_export))
        .routes(routes!(list_archives, create_archive))
        .with_state(service)
}

// ---- Mẫu in -------------------------------------------------------------------------------------

/// A print template of a document kind and language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesTemplate)]
pub(crate) struct TemplateResponse {
    id: Uuid,
    /// The unit the template is for; `null`: every unit of the tenant.
    org_id: Option<Uuid>,
    kind: String,
    /// `en` or `vi`.
    locale: String,
    /// Goes up by one on each change.
    version: i32,
    /// The template (Jinja syntax; see `GET /api/v1/invoices/template-defaults`).
    body: String,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<template::Model> for TemplateResponse {
    fn from(row: template::Model) -> Self {
        TemplateResponse {
            id: row.id,
            org_id: (!row.org_id.is_nil()).then_some(row.org_id),
            kind: row.kind,
            locale: row.locale,
            version: row.version,
            body: row.body,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A print template to save.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesTemplateRequest)]
pub(crate) struct TemplateRequest {
    /// The unit; omit for every unit of the tenant.
    org_id: Option<Uuid>,
    /// Document kind, e.g. `retail_receipt`, `delivery_note`, `goods_receipt`, `cash_receipt`.
    kind: String,
    /// `en` or `vi`.
    locale: String,
    /// The template, at most `AUTOXIDE_TEMPLATE_MAX_BYTES` bytes.
    body: String,
}

/// A new body for a print template.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesTemplateChange)]
pub(crate) struct TemplateChange {
    body: String,
}

const TEMPLATE_422: &str = "A value is invalid; every problem in `errors`. Problems in `body` have the code `template.syntax_error`, `template.unknown_variable` or `template.required_variable_missing` and give the line and column; a template that is too long has the code `template.limit_exceeded`.";

/// List the print templates of the tenant.
///
/// Requires `invoices.read`. Documents without a template of their own use the tenant's, or the
/// built-in one (`GET /api/v1/invoices/template-defaults`).
#[utoipa::path(
    get,
    path = "/templates",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "list_invoice_templates",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of templates.", body = Page<TemplateResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_templates(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<TemplateResponse>, Error> {
    principal.require(&READ)?;
    let page = service.list_templates(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(TemplateResponse::from))
}

/// Save a print template.
///
/// Requires `invoices.templates_manage` on the unit (on the tenant's root for a template of every
/// unit). The template may use only the variables of `GET /api/v1/invoices/template-defaults` and
/// must use `document.number`; every value it prints is HTML-escaped. One template per unit, kind
/// and language.
#[utoipa::path(
    post,
    path = "/templates",
    extensions(("x-permissions" = json!(["invoices.templates_manage"]))),
    operation_id = "create_invoice_template",
    security(("bearer" = [])),
    request_body = TemplateRequest,
    responses(
        (status = 201, description = "The template.", body = Data<TemplateResponse>, headers(("Location" = String, description = "Path of the template."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.templates_manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The unit is outside your scope; code `invoices.template_not_found`.", body = Problem),
        (status = 409, description = "The unit has a template for this kind and language; code `invoices.template_exists`.", body = Problem),
        (status = 422, description = TEMPLATE_422, body = Problem),
    )
)]
async fn create_template(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<TemplateRequest>,
) -> Result<Created<TemplateResponse>, Error> {
    let row = service
        .create_template(
            &tenant,
            &principal,
            TemplateInput {
                org: input.org_id.map(OrgId::from_uuid),
                kind: input.kind,
                locale: input.locale,
                body: input.body,
            },
        )
        .await?;
    Ok(Created::new(format!("/invoices/templates/{}", row.id), row.into()))
}

/// Get a print template.
///
/// Requires `invoices.read`.
#[utoipa::path(
    get,
    path = "/templates/{id}",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "get_invoice_template",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the template.")),
    responses(
        (status = 200, description = "The template.", body = Data<TemplateResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such template; code `invoices.template_not_found`.", body = Problem),
    )
)]
async fn get_template(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TemplateResponse>, Error> {
    Ok(Data { data: service.get_template(&tenant, &principal, id).await?.into() })
}

/// Change a print template.
///
/// Requires `invoices.templates_manage` on the template's unit. Checked like a new template;
/// `version` goes up by one.
#[utoipa::path(
    put,
    path = "/templates/{id}",
    extensions(("x-permissions" = json!(["invoices.templates_manage"]))),
    operation_id = "update_invoice_template",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the template.")),
    request_body = TemplateChange,
    responses(
        (status = 200, description = "The template after the change.", body = Data<TemplateResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.templates_manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such template in your scope; code `invoices.template_not_found`.", body = Problem),
        (status = 422, description = TEMPLATE_422, body = Problem),
    )
)]
async fn update_template(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<TemplateChange>,
) -> Result<Data<TemplateResponse>, Error> {
    Ok(Data { data: service.update_template(&tenant, &principal, id, input.body).await?.into() })
}

/// Delete a print template.
///
/// Requires `invoices.templates_manage` on the template's unit. Documents then print with the
/// tenant's template or the built-in one.
#[utoipa::path(
    delete,
    path = "/templates/{id}",
    extensions(("x-permissions" = json!(["invoices.templates_manage"]))),
    operation_id = "delete_invoice_template",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the template.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.templates_manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such template in your scope; code `invoices.template_not_found`.", body = Problem),
    )
)]
async fn delete_template(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_template(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A template body to try.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesTemplatePreviewRequest)]
pub(crate) struct PreviewRequest {
    kind: String,
    /// `en` or `vi`.
    locale: String,
    body: String,
    /// A document to print with it; omit to use a built-in sample document.
    document_id: Option<Uuid>,
}

/// The HTML a template gives.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = InvoicesTemplatePreview)]
pub(crate) struct PreviewResponse {
    html: String,
}

/// Try a print template without saving it.
///
/// Requires `invoices.templates_manage` (and `invoices.read` on the document's unit when
/// `document_id` is given). Checked like a saved template.
#[utoipa::path(
    post,
    path = "/templates/preview",
    extensions(("x-permissions" = json!(["invoices.templates_manage"]))),
    operation_id = "preview_invoice_template",
    security(("bearer" = [])),
    request_body = PreviewRequest,
    responses(
        (status = 200, description = "The HTML.", body = Data<PreviewResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.templates_manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document in your scope; code `invoices.document_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid, or the template fails while printing (`template.render_failed`, `template.limit_exceeded`); every problem in `errors`.", body = Problem),
    )
)]
async fn preview_template(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PreviewRequest>,
) -> Result<Data<PreviewResponse>, Error> {
    let html = service
        .preview_template(
            &tenant,
            &principal,
            &input.kind,
            &input.locale,
            &input.body,
            input.document_id,
        )
        .await?;
    Ok(Data { data: PreviewResponse { html } })
}

/// A variable a print template may use.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = InvoicesTemplateVariable)]
pub(crate) struct VariableResponse {
    /// Dotted path, e.g. `document.number`; a declared object allows all its fields.
    name: String,
    /// The template must use it.
    required: bool,
    description: String,
}

/// The built-in print template and the variables templates may use.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = InvoicesTemplateDefaults)]
pub(crate) struct DefaultsResponse {
    /// The built-in template, for every kind and language (words come from `labels`).
    body: String,
    variables: Vec<VariableResponse>,
    kinds: Vec<String>,
    locales: Vec<String>,
    /// Filters: `money` (an `{amount, currency}` value), `number` (a decimal string), `date`
    /// (`YYYY-MM-DD`) format in the language of the print; built-in Jinja filters and tests work too.
    /// There is no `safe` filter: every value is HTML-escaped.
    filters: Vec<String>,
}

/// Get the built-in print template and the variables templates may use.
///
/// Requires `invoices.read`. Templates use Jinja syntax (`{{ … }}`, `{% if %}`, `{% for %}`), run in
/// a sandbox (no files, no network, no other templates), with limits on size, steps and output.
#[utoipa::path(
    get,
    path = "/template-defaults",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "get_invoice_template_defaults",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The built-in template and the variables.", body = Data<DefaultsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn template_defaults(principal: Principal) -> Result<Data<DefaultsResponse>, Error> {
    principal.require(&READ)?;
    let variables = print::schema()
        .variables
        .into_iter()
        .map(|variable| VariableResponse {
            name: variable.path.to_owned(),
            required: variable.required,
            description: variable.description.to_owned(),
        })
        .collect();
    Ok(Data {
        data: DefaultsResponse {
            body: DEFAULT_TEMPLATE.to_owned(),
            variables,
            kinds: KINDS.iter().map(|kind| (*kind).to_owned()).collect(),
            locales: LOCALES.iter().map(|locale| (*locale).to_owned()).collect(),
            filters: vec!["money".to_owned(), "number".to_owned(), "date".to_owned()],
        },
    })
}

// ---- Bản in -------------------------------------------------------------------------------------

/// How to print.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RenderQuery {
    /// `html` (default) or `pdf`.
    format: Option<String>,
    /// `en` or `vi`; default the language of `Accept-Language` (English when none matches).
    locale: Option<String>,
}

fn file_response(rendered: Rendered, disposition: Disposition) -> Response {
    let mut response = Body::from(rendered.body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(rendered.content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&content_disposition(disposition, &rendered.file_name))
    {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// Print a document as HTML or PDF.
///
/// Requires `invoices.read` on the unit. HTML comes from the unit's template, the tenant's template
/// or the built-in one, for the document's kind and language. PDF has a fixed layout built from the
/// same data (title, parties, lines, totals, signatures) with fonts that carry Vietnamese marks;
/// templates change the HTML only — print the HTML from a browser for a PDF in your own layout.
#[utoipa::path(
    get,
    path = "/documents/{id}/render",
    extensions(("x-permissions" = json!(["invoices.read"]))),
    operation_id = "render_invoice_document",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the document."), RenderQuery),
    responses(
        (status = 200, description = "The print: `text/html` or `application/pdf`.", content_type = "application/octet-stream", body = Vec<u8>, headers(("Content-Disposition" = String, description = "`inline` with a file name such as `retail_receipt-12.pdf`."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such document in your scope; code `invoices.document_not_found`.", body = Problem),
        (status = 422, description = "`format` or `locale` is invalid, or the saved template fails while printing (`template.render_failed`, `template.limit_exceeded`).", body = Problem),
    )
)]
async fn render_document(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    language: autoxide_kernel::i18n::Language,
    Path(id): Path<Uuid>,
    Query(query): Query<RenderQuery>,
) -> Result<Response, Error> {
    principal.require(&READ)?;
    let mut problems = Vec::new();
    let format = match query.format.as_deref().unwrap_or("html") {
        "html" => Some(RenderFormat::Html),
        "pdf" => Some(RenderFormat::Pdf),
        _ => {
            problems.push(FieldError::new("format", "invalid", "must be `html` or `pdf`"));
            None
        }
    };
    let locale = query.locale.unwrap_or_else(|| language.tag().to_owned());
    if !LOCALES.contains(&locale.as_str()) {
        problems.push(FieldError::new("locale", "invalid", "must be `en` or `vi`"));
    }
    let (Some(format), true) = (format, problems.is_empty()) else {
        return Err(Error::validation(problems));
    };
    let rendered = service.render_manual(&tenant, &principal, id, format, &locale).await?;
    Ok(file_response(rendered, Disposition::Inline))
}

// ---- Xuất kế toán -------------------------------------------------------------------------------

/// An export profile (file layout).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = InvoicesExportProfile)]
pub(crate) struct ProfileResponse {
    name: String,
    description: String,
    formats: Vec<String>,
}

/// List the export profiles.
///
/// Requires `invoices.export`.
#[utoipa::path(
    get,
    path = "/export-profiles",
    extensions(("x-permissions" = json!(["invoices.export"]))),
    operation_id = "list_invoice_export_profiles",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The profiles.", body = Data<Vec<ProfileResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.export`; code `permission_denied`.", body = Problem),
    )
)]
async fn export_profiles(principal: Principal) -> Result<Data<Vec<ProfileResponse>>, Error> {
    principal.require(&EXPORT)?;
    let data = PROFILES
        .iter()
        .map(|(name, description)| ProfileResponse {
            name: (*name).to_owned(),
            description: (*description).to_owned(),
            formats: FORMATS.iter().map(|format| (*format).to_owned()).collect(),
        })
        .collect();
    Ok(Data { data })
}

/// An accounting export.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesExport)]
pub(crate) struct ExportResponse {
    id: Uuid,
    /// Only the book of this unit; `null`: every book of the tenant.
    org_id: Option<Uuid>,
    profile: String,
    /// `csv` or `xlsx`.
    format: String,
    period_from: Date,
    period_to: Date,
    /// `pending`, `running`, `ready` (download it) or `failed` (ask for a new one).
    status: String,
    size: Option<i64>,
    /// Rows in the file, without the header row.
    row_count: Option<i64>,
    error: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<export::Model> for ExportResponse {
    fn from(row: export::Model) -> Self {
        ExportResponse {
            id: row.id,
            org_id: (!row.org_id.is_nil()).then_some(row.org_id),
            profile: row.profile,
            format: row.format,
            period_from: row.period_from,
            period_to: row.period_to,
            status: row.status,
            size: row.size,
            row_count: row.row_count,
            error: row.error,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// An export to make.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InvoicesExportRequest)]
pub(crate) struct ExportRequest {
    /// Only the book (or documents) of this unit; omit for the whole tenant.
    org_id: Option<Uuid>,
    /// See `GET /api/v1/invoices/export-profiles`.
    #[schema(example = "generic_journal")]
    profile: String,
    /// `csv` (UTF-8 with BOM, opens in Excel) or `xlsx`.
    format: String,
    /// First day, `YYYY-MM-DD`.
    from: Date,
    /// Last day, `YYYY-MM-DD`; at most 366 days after `from`.
    to: Date,
}

/// Export the general journal or the documents of a period.
///
/// Requires `invoices.export`. The file is made by a background job: follow the `Location` until
/// `status` is `ready`, then download it. For the accountant's software: account numbers follow the
/// account's own code or the Vietnamese chart of accounts (needs checking by an accountant).
#[utoipa::path(
    post,
    path = "/exports",
    extensions(("x-permissions" = json!(["invoices.export"]))),
    operation_id = "create_invoice_export",
    security(("bearer" = [])),
    request_body = ExportRequest,
    responses(
        (status = 201, description = "Asked for; the job makes the file shortly.", body = Data<ExportResponse>, headers(("Location" = String, description = "Path of the export."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.export`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
        (status = 503, description = "No file storage is set up; code `invoices.storage_unavailable`.", body = Problem),
    )
)]
async fn create_export(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ExportRequest>,
) -> Result<Created<ExportResponse>, Error> {
    let row = service
        .request_export(
            &tenant,
            &principal,
            ExportInput {
                org: input.org_id.map(OrgId::from_uuid),
                profile: input.profile,
                format: input.format,
                from: input.from,
                to: input.to,
            },
        )
        .await?;
    Ok(Created::new(format!("/invoices/exports/{}", row.id), row.into()))
}

/// List accounting exports.
///
/// Requires `invoices.export`.
#[utoipa::path(
    get,
    path = "/exports",
    extensions(("x-permissions" = json!(["invoices.export"]))),
    operation_id = "list_invoice_exports",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of exports.", body = Page<ExportResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.export`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_exports(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<ExportResponse>, Error> {
    principal.require(&EXPORT)?;
    let page = service.list_exports(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(ExportResponse::from))
}

/// Get an accounting export.
///
/// Requires `invoices.export`.
#[utoipa::path(
    get,
    path = "/exports/{id}",
    extensions(("x-permissions" = json!(["invoices.export"]))),
    operation_id = "get_invoice_export",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The export.", body = Data<ExportResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.export`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such export; code `invoices.export_not_found`.", body = Problem),
    )
)]
async fn get_export(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ExportResponse>, Error> {
    Ok(Data { data: service.get_export(&tenant, &principal, id).await?.into() })
}

/// Make a failed accounting export again.
///
/// Requires `invoices.export`. Puts the export back to `pending` with the same profile, format and
/// period, and starts the job again; follow it with `GET /api/v1/invoices/exports/{id}`. Safe to call
/// again: an export already `pending` or `running` is returned as it is.
#[utoipa::path(
    post,
    path = "/exports/{id}/retry",
    extensions(("x-permissions" = json!(["invoices.export"]))),
    operation_id = "retry_invoice_export",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The export, `pending` again (or still `pending` / `running`).", body = Data<ExportResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.export`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such export; code `invoices.export_not_found`.", body = Problem),
        (status = 409, description = "The export is `ready`; ask for a new one. Code `invoices.export_not_retryable`.", body = Problem),
        (status = 503, description = "No file storage is set up; code `invoices.storage_unavailable`.", body = Problem),
    )
)]
async fn retry_export(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ExportResponse>, Error> {
    Ok(Data { data: service.retry_export(&tenant, &principal, id).await?.into() })
}

/// Download the file of an accounting export.
///
/// Requires `invoices.export`.
#[utoipa::path(
    get,
    path = "/exports/{id}/download",
    extensions(("x-permissions" = json!(["invoices.export"]))),
    operation_id = "download_invoice_export",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The file (`text/csv` or the XLSX type).", content_type = "application/octet-stream", body = Vec<u8>, headers(("Content-Disposition" = String, description = "`attachment` with a file name such as `generic_journal-2026-01-01-2026-01-31.csv`."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.export`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such export; code `invoices.export_not_found`.", body = Problem),
        (status = 409, description = "The file is not ready (`invoices.export_not_ready`, its status in the message).", body = Problem),
    )
)]
async fn download_export(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    let (row, file) = service.export_file(&tenant, &principal, id).await?;
    let name = format!("{}-{}-{}.{}", row.profile, row.period_from, row.period_to, row.format);
    let kind = if row.format == "xlsx" {
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    } else {
        "text/csv; charset=utf-8"
    };
    let mut response = Body::from_stream(file.content).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(file.size));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&content_disposition(Disposition::Attachment, &name)) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

// ---- Bản lưu trữ --------------------------------------------------------------------------------

/// An archive copy of the tenant's accounting records.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = InvoicesArchive)]
pub(crate) struct ArchiveResponse {
    id: Uuid,
    /// `ready` or `failed`.
    status: String,
    /// `manual` or `tenant_deleted`.
    reason: String,
    /// Where the files are in the shared file storage (outside the tenant's folder).
    storage_prefix: String,
    /// SHA-256 of `manifest.json`, which lists the SHA-256 of every file.
    manifest_sha256: Option<String>,
    /// Keep the archive at least until this day.
    retain_until: Option<Date>,
    created_at: Timestamp,
}

impl From<archive::Model> for ArchiveResponse {
    fn from(row: archive::Model) -> Self {
        ArchiveResponse {
            id: row.id,
            status: row.status,
            reason: row.reason,
            storage_prefix: row.storage_prefix,
            manifest_sha256: row.manifest_sha256,
            retain_until: row.retain_until,
            created_at: row.created_at,
        }
    }
}

/// Make an archive copy of the accounting records.
///
/// Requires `invoices.archive`. Writes the general journal (`ledger/journal.csv`), every document
/// (`documents.jsonl`) and `manifest.json` (SHA-256 of each file) to the shared file storage, outside
/// the tenant's folder: the copy stays after the tenant is deleted. Accounting records must be kept
/// at least 10 years (Accounting Law 88/2015, Article 41).
#[utoipa::path(
    post,
    path = "/archives",
    extensions(("x-permissions" = json!(["invoices.archive"]))),
    operation_id = "create_invoice_archive",
    security(("bearer" = [])),
    responses(
        (status = 201, description = "The archive.", body = Data<ArchiveResponse>, headers(("Location" = String, description = "Path of the archive list."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.archive`; code `permission_denied`.", body = Problem),
        (status = 503, description = "No file storage is set up; code `invoices.storage_unavailable`.", body = Problem),
    )
)]
async fn create_archive(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Created<ArchiveResponse>, Error> {
    let row = service.archive_manual(&tenant, &principal).await?;
    Ok(Created::new("/invoices/archives".to_owned(), row.into()))
}

/// List archive copies of the accounting records.
///
/// Requires `invoices.archive`.
#[utoipa::path(
    get,
    path = "/archives",
    extensions(("x-permissions" = json!(["invoices.archive"]))),
    operation_id = "list_invoice_archives",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of archives.", body = Page<ArchiveResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `invoices.archive`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_archives(
    State(service): State<Arc<InvoicesService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<ArchiveResponse>, Error> {
    principal.require(&ARCHIVE)?;
    let page = service.list_archives(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(ArchiveResponse::from))
}
