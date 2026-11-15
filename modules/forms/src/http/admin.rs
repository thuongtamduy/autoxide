//! Endpoint quản lý form (`/`, `/{id}`). Đọc cần `forms.read`; ghi cần `forms.manage`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::{choice, page_request, present};
use crate::forms::fields::{Field, Localized};
use crate::forms::{Form, FormInput, FormStatus};
use crate::service::FormsService;
use crate::submissions::inbox::SubmissionCounts;

/// A form, as admins see it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct FormResponse {
    pub id: Uuid,
    /// Unique; part of the public address `/api/v1/forms/public/{key}`.
    #[schema(example = "contact")]
    pub key: String,
    /// Name for admins (also shown by the public endpoint).
    #[schema(example = "Contact us")]
    pub name: String,
    pub status: FormStatus,
    /// Version of `fields`: grows by one each time the fields change. Each submission keeps
    /// the version it was sent with, and is shown with that version's labels.
    pub version: i32,
    pub fields: Vec<Field>,
    /// Websites allowed to send the form from a browser (`https://example.com`); empty: any.
    /// A request whose `Origin` is not listed is refused with `forms.origin_not_allowed`.
    pub allowed_origins: Vec<String>,
    /// Shown to the sender after sending.
    pub success_message: Localized,
    /// Addresses mailed for each new submission. Never shown by the public endpoint.
    pub notify_emails: Vec<String>,
    /// Key of the `email` field used as `Reply-To` of the notification and to find
    /// submissions by email; `null`: the first `email` field.
    pub reply_to_field: Option<String>,
    /// Public submissions per hour from one network block; `null`: the server default.
    pub ip_limit_per_hour: Option<i32>,
    /// Public submissions per hour from one email address; `null`: the server default.
    pub email_limit_per_hour: Option<i32>,
    /// Days submissions are kept (`0`: until deleted); `null`: the server default.
    pub retention_days: Option<i32>,
    /// The `scope` to ask `GET /api/v1/antispam/token` for, e.g. `forms:contact`.
    #[schema(example = "forms:contact")]
    pub token_scope: String,
    /// Submissions by status (`new`, `read`, `archived`, `spam`), in the list and when one form is
    /// read; absent in the answers of a create or a change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submissions: Option<SubmissionCounts>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Form> for FormResponse {
    fn from(form: Form) -> Self {
        FormResponse {
            token_scope: form.scope(),
            status: form.status(),
            id: form.row.id,
            key: form.row.key,
            name: form.row.name,
            version: form.row.version,
            fields: form.fields,
            allowed_origins: form.allowed_origins,
            success_message: form.success_message,
            notify_emails: form.notify_emails,
            reply_to_field: form.row.reply_to_field,
            ip_limit_per_hour: form.row.ip_limit_per_hour,
            email_limit_per_hour: form.row.email_limit_per_hour,
            retention_days: form.row.retention_days,
            submissions: None,
            created_at: form.row.created_at,
            updated_at: form.row.updated_at,
        }
    }
}

/// A new form.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateFormRequest {
    /// Lower-case letters and digits in groups joined by `-`, at most 50 characters; unique.
    #[schema(example = "contact")]
    key: String,
    /// 1 to 200 characters.
    #[schema(example = "Contact us")]
    name: String,
    /// Omitted: `open`.
    status: Option<FormStatus>,
    /// 1 to `AUTOXIDE_FORMS_MAX_FIELDS` (default 50) fields, in display order.
    fields: Vec<Field>,
    /// Up to 20 origins like `https://example.com`.
    allowed_origins: Option<Vec<String>>,
    /// Up to 1000 characters per language.
    success_message: Option<Localized>,
    /// Up to 20 addresses mailed for each new submission (the mails carry the answers).
    /// Setting them also needs the permission `forms.read`.
    notify_emails: Option<Vec<String>>,
    /// Key of an `email` field.
    reply_to_field: Option<String>,
    /// 1 to 100000.
    ip_limit_per_hour: Option<u32>,
    /// 1 to 100000.
    email_limit_per_hour: Option<u32>,
    /// 0 (keep) to 36500.
    retention_days: Option<u32>,
}

impl From<CreateFormRequest> for FormInput {
    fn from(input: CreateFormRequest) -> Self {
        FormInput {
            key: Some(input.key),
            name: Some(input.name),
            status: input.status,
            fields: Some(input.fields),
            allowed_origins: input.allowed_origins,
            success_message: input.success_message,
            notify_emails: input.notify_emails,
            reply_to_field: input.reply_to_field.map(Some),
            ip_limit_per_hour: input.ip_limit_per_hour.map(Some),
            email_limit_per_hour: input.email_limit_per_hour.map(Some),
            retention_days: input.retention_days.map(Some),
        }
    }
}

/// Changes to a form. Omitted fields stay as they are; `null` clears the nullable ones.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateFormRequest {
    key: Option<String>,
    name: Option<String>,
    status: Option<FormStatus>,
    /// The whole new list of fields; a change makes a new `version`. Old submissions keep
    /// the labels of their own version.
    fields: Option<Vec<Field>>,
    allowed_origins: Option<Vec<String>>,
    success_message: Option<Localized>,
    /// The whole new list of addresses mailed for each new submission (the mails carry the
    /// answers). Changing it also needs the permission `forms.read`.
    notify_emails: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    reply_to_field: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<u32>, nullable)]
    ip_limit_per_hour: Option<Option<u32>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<u32>, nullable)]
    email_limit_per_hour: Option<Option<u32>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<u32>, nullable)]
    retention_days: Option<Option<u32>>,
}

impl From<UpdateFormRequest> for FormInput {
    fn from(input: UpdateFormRequest) -> Self {
        FormInput {
            key: input.key,
            name: input.name,
            status: input.status,
            fields: input.fields,
            allowed_origins: input.allowed_origins,
            success_message: input.success_message,
            notify_emails: input.notify_emails,
            reply_to_field: input.reply_to_field,
            ip_limit_per_hour: input.ip_limit_per_hour,
            email_limit_per_hour: input.email_limit_per_hour,
            retention_days: input.retention_days,
        }
    }
}

/// Which forms to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FormsQuery {
    /// `open` or `closed`; omitted: both.
    #[param(value_type = Option<FormStatus>, inline)]
    pub status: Option<String>,
}

/// List forms.
///
/// Requires `forms.read`. Newest first; follow `pagination.next_cursor` for more.
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-permissions" = json!(["forms.read"]))),
    operation_id = "list_forms",
    security(("bearer" = [])),
    params(FormsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of forms.", body = Page<FormResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_forms(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<FormsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<FormResponse>, Error> {
    let mut problems = Vec::new();
    let status = choice(query.status.as_deref(), "status", "`open`, `closed`", &mut problems);
    let page = page_request(paging, problems)?;
    counted(&service, &tenant, service.list_forms(&tenant, &principal, status, &page).await?).await
}

/// Một trang form như API trả về, kèm số bài gửi theo trạng thái (một câu đếm cho cả trang).
pub(crate) async fn counted(
    service: &FormsService,
    tenant: &TenantContext,
    page: Page<Form>,
) -> Result<Page<FormResponse>, Error> {
    let ids: Vec<Uuid> = page.data.iter().map(|form| form.row.id).collect();
    let counts = service.submission_counts(tenant, &ids).await?;
    Ok(page.map(|form| {
        let submissions = Some(counts.get(&form.row.id).copied().unwrap_or_default());
        FormResponse { submissions, ..FormResponse::from(form) }
    }))
}

/// Create a form.
///
/// Requires `forms.manage`, and also `forms.read` when `notify_emails` is not empty (the
/// notification mails carry the answers). Every problem of the definition is reported at once
/// in `errors` (paths like `fields[2].options`). The form takes submissions at once unless
/// `status` is `closed`. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/",
    extensions(("x-permissions" = json!(["forms.manage", "forms.read"]))),
    operation_id = "create_form",
    security(("bearer" = [])),
    request_body = CreateFormRequest,
    responses(
        (status = 201, description = "Created.", body = Data<FormResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.manage` (or `forms.read`, needed to set `notify_emails`); code `permission_denied`.", body = Problem),
        (status = 409, description = "The key is taken; code `forms.key_taken`.", body = Problem),
        (status = 422, description = "The definition is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_form(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateFormRequest>,
) -> Result<Created<FormResponse>, Error> {
    let form = service.create_form(&tenant, &principal, input.into()).await?;
    Ok(Created::new(format!("/forms/{}", form.row.id), form.into()))
}

/// Get a form.
///
/// Requires `forms.read`.
#[utoipa::path(
    get,
    path = "/{id}",
    extensions(("x-permissions" = json!(["forms.read"]))),
    operation_id = "get_form",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the form.")),
    responses(
        (status = 200, description = "The form.", body = Data<FormResponse>, headers(("ETag" = String, description = "Version of the form; send it in `If-Match` to change it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No form has this id; code `forms.not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_form(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<FormResponse>, Error> {
    let form = service.get_form(&tenant, &principal, id).await?;
    let counts = service.submission_counts(&tenant, &[form.row.id]).await?;
    let submissions = Some(counts.get(&form.row.id).copied().unwrap_or_default());
    Ok(Tagged { version: form.row.updated_at, data: FormResponse { submissions, ..form.into() } })
}

/// Change a form.
///
/// Requires `forms.manage`, and also `forms.read` to change `notify_emails` (the notification
/// mails carry the answers). `If-Match` is required. Sending `fields` replaces the whole list;
/// a change makes a new `version` (old submissions keep their own labels). Close a form with
/// `{"status": "closed"}`.
#[utoipa::path(
    patch,
    path = "/{id}",
    extensions(("x-permissions" = json!(["forms.manage", "forms.read"]))),
    operation_id = "update_form",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the form."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = UpdateFormRequest,
    responses(
        (status = 200, description = "The form after the change.", body = Data<FormResponse>, headers(("ETag" = String, description = "New version of the form."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.manage` (or `forms.read`, needed to change `notify_emails`); code `permission_denied`.", body = Problem),
        (status = 404, description = "No form has this id; code `forms.not_found`.", body = Problem),
        (status = 409, description = "The key is taken; code `forms.key_taken`.", body = Problem),
        (status = 412, description = "The form changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_form(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateFormRequest>,
) -> Result<Tagged<FormResponse>, Error> {
    let expected = if_match.required()?;
    let form = service.update_form(&tenant, &principal, id, input.into(), expected).await?;
    Ok(Tagged { version: form.row.updated_at, data: form.into() })
}

/// Delete a form.
///
/// Requires `forms.manage`. `If-Match` is required. Deletes every submission of the form
/// too; export them first if you need them. To stop taking submissions but keep them, close
/// the form instead.
#[utoipa::path(
    delete,
    path = "/{id}",
    extensions(("x-permissions" = json!(["forms.manage"]))),
    operation_id = "delete_form",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the form."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    responses(
        (status = 204, description = "Deleted, with its submissions."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No form has this id; code `forms.not_found`.", body = Problem),
        (status = 412, description = "The form changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn delete_form(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<StatusCode, Error> {
    let expected = if_match.required()?;
    service.delete_form(&tenant, &principal, id, expected).await?;
    Ok(StatusCode::NO_CONTENT)
}
