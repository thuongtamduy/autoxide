//! Endpoint hộp thư: bài gửi của một form, một bài, xuất CSV, xuất / xoá theo email.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::{Disposition, ValidJson, content_disposition};
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::{choice, page_request};
use crate::forms::fields::{FieldOption, FieldType, Localized};
use crate::service::FormsService;
use crate::submissions::inbox::{InboxFilter, Submission, SubmissionStatus};

/// One answer, with the field as it was when the submission was sent.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct AnswerResponse {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: FieldType,
    /// The field's label in the form version the submission was sent with.
    pub label: Localized,
    /// A string; a number for `number`; an array of values for `checkbox`; `true` for
    /// `consent`.
    #[schema(schema_with = answer_value)]
    pub value: Value,
    /// For `select`, `radio` and `checkbox`: the chosen options with their labels in the form
    /// version the submission was sent with (in the order of `value`). A value no longer among
    /// that version's options comes with an empty label. Absent for other types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<FieldOption>>,
}

/// Lược đồ của `value`: một trong bốn kiểu (thay cho "kiểu nào cũng được" của `Value`), để
/// client sinh kiểu đúng — `string | number | string[] | boolean`.
fn answer_value() -> utoipa::openapi::schema::Schema {
    use utoipa::openapi::schema::{ArrayBuilder, ObjectBuilder, OneOfBuilder, Type};
    let of = |kind: Type| ObjectBuilder::new().schema_type(kind).build();
    OneOfBuilder::new()
        .item(of(Type::String))
        .item(of(Type::Number))
        .item(ArrayBuilder::new().items(of(Type::String)).build())
        .item(of(Type::Boolean))
        .description(Some(
            "A string; a number for `number`; an array of values for `checkbox`; `true` for `consent`.",
        ))
        .build()
        .into()
}

/// Các lựa chọn đã chọn của một câu trả lời, kèm nhãn của đúng phiên bản form lúc gửi.
fn chosen(field: &crate::forms::fields::Field, value: &Value) -> Option<Vec<FieldOption>> {
    if !field.kind.has_options() {
        return None;
    }
    let values: Vec<&str> = match value {
        Value::String(one) => vec![one.as_str()],
        Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    Some(
        values
            .into_iter()
            .map(|picked| {
                field.options.iter().find(|option| option.value == picked).cloned().unwrap_or(
                    FieldOption { value: picked.to_owned(), label: Localized::default() },
                )
            })
            .collect(),
    )
}

/// A submission.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SubmissionResponse {
    pub id: Uuid,
    pub form_id: Uuid,
    pub form_key: String,
    /// The form version (of its fields) the submission was sent with.
    pub form_version: i32,
    pub status: SubmissionStatus,
    /// Why the anti-spam checks held it (`too_fast`, `duplicate`, `classifier`…, joined by
    /// commas); `null` unless `spam`.
    pub spam_reason: Option<String>,
    /// The answers, by field key.
    #[schema(value_type = Object)]
    pub data: Map<String, Value>,
    /// The answers in the order of the form version, with their labels.
    pub answers: Vec<AnswerResponse>,
    /// The sender's email address (lower case), when the form asks for one.
    pub email: Option<String>,
    /// Language the sender saw: `en` or `vi`.
    pub locale: Option<String>,
    /// Page the form was sent from.
    pub page_url: Option<String>,
    /// Page that led the sender to the form, when the website sends it.
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
    /// Keyed hash of the sender's network block (never the address itself); cleared after
    /// `AUTOXIDE_FORMS_IP_RETENTION_DAYS`.
    pub ip_fingerprint: Option<String>,
    /// Proof of consent when the form has a `consent` field: `{text, version, at, source,
    /// field, ip_fingerprint}` — the exact text shown, the form version, when.
    #[schema(value_type = Option<Object>)]
    pub consent: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Submission> for SubmissionResponse {
    fn from(found: Submission) -> Self {
        let answers = found
            .fields
            .iter()
            .filter_map(|field| {
                found.data.get(&field.key).map(|value| AnswerResponse {
                    key: field.key.clone(),
                    kind: field.kind,
                    label: field.label.clone(),
                    choices: chosen(field, value),
                    value: value.clone(),
                })
            })
            .collect();
        let status = found.status();
        let row = found.row;
        SubmissionResponse {
            id: row.id,
            form_id: row.form_id,
            form_key: found.form_key,
            form_version: row.form_version,
            status,
            spam_reason: row.spam_reason,
            data: found.data,
            answers,
            email: row.email,
            locale: row.locale,
            page_url: row.page_url,
            referrer: row.referrer,
            user_agent: row.user_agent,
            ip_fingerprint: row.ip_fingerprint,
            consent: found.consent,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Which submissions to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SubmissionsQuery {
    /// `new`, `read`, `archived` or `spam`; omitted: every status but `spam`.
    #[param(value_type = Option<SubmissionStatus>, inline)]
    pub status: Option<String>,
    /// Only submissions whose answers contain this text, ignoring case and accents.
    pub q: Option<String>,
}

impl SubmissionsQuery {
    pub(crate) fn filter(&self, problems: &mut Vec<FieldError>) -> InboxFilter {
        InboxFilter {
            status: choice(self.status.as_deref(), "status", STATUSES, problems),
            search: self.q.clone(),
        }
    }
}

pub(crate) const STATUSES: &str = "`new`, `read`, `archived`, `spam`";

/// List the submissions of a form.
///
/// Requires `forms.read`. Newest first; follow `pagination.next_cursor` with the same filters
/// for more. Each submission comes with the labels of the form version it was sent with.
#[utoipa::path(
    get,
    path = "/{id}/submissions",
    extensions(("x-permissions" = json!(["forms.read"]))),
    operation_id = "list_form_submissions",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the form."), SubmissionsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of submissions.", body = Page<SubmissionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No form has this id; code `forms.not_found`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_submissions(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<SubmissionsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<SubmissionResponse>, Error> {
    let mut problems = Vec::new();
    let filter = query.filter(&mut problems);
    let page = page_request(paging, problems)?;
    Ok(service
        .list_submissions(&tenant, &principal, id, &filter, &page)
        .await?
        .map(SubmissionResponse::from))
}

/// What to export.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ExportQuery {
    /// `csv` (the only format, and the default).
    #[param(example = "csv")]
    pub format: Option<String>,
    /// `new`, `read`, `archived` or `spam`; omitted: every status but `spam`.
    #[param(value_type = Option<SubmissionStatus>, inline)]
    pub status: Option<String>,
    /// Only these submissions: up to 200 ids, separated by commas (e.g. the rows picked in a
    /// list). Picked submissions are exported whatever their status, `spam` too, unless `status`
    /// is also given.
    pub ids: Option<String>,
}

/// Export the submissions of a form as CSV.
///
/// Requires `forms.export`. One row per submission, oldest first; columns `id`,
/// `created_at`, `status`, `form_version`, `locale`, `email`, `page_url`, `referrer`, then
/// one column per field key (current fields first, then fields only older versions had).
/// UTF-8 with a byte order mark, CRLF line ends, every cell quoted. Cells starting with `=`,
/// `+`, `-`, `@`, tab or carriage return get a leading `'` so spreadsheets do not run them as
/// formulas.
#[utoipa::path(
    get,
    path = "/{id}/submissions/export",
    extensions(("x-permissions" = json!(["forms.export"]))),
    operation_id = "export_form_submissions",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the form."), ExportQuery),
    responses(
        (status = 200, description = "The CSV file.", body = String, content_type = "text/csv", headers(("Content-Disposition" = String, description = "`attachment` with the file name `<key>-submissions.csv`."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.export`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No form has this id; code `forms.not_found`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn export_submissions(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, Error> {
    let mut problems = Vec::new();
    if query.format.as_deref().is_some_and(|format| format.trim() != "csv") {
        problems.push(FieldError::new("format", "invalid", "must be `csv`"));
    }
    let status = choice(query.status.as_deref(), "status", STATUSES, &mut problems);
    let ids = picked_ids(query.ids.as_deref(), &mut problems);
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let (name, text) = service.export_csv(&tenant, &principal, id, status, &ids).await?;
    let mut response = text.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(&content_disposition(Disposition::Attachment, &name)) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

/// Get a submission.
///
/// Requires `forms.read`.
#[utoipa::path(
    get,
    path = "/submissions/{id}",
    extensions(("x-permissions" = json!(["forms.read"]))),
    operation_id = "get_form_submission",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the submission.")),
    responses(
        (status = 200, description = "The submission.", body = Data<SubmissionResponse>, headers(("ETag" = String, description = "Version of the submission; send it in `If-Match` to change its status."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No submission has this id; code `forms.submission_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_submission(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<SubmissionResponse>, Error> {
    let found = service.get_submission(&tenant, &principal, id).await?;
    Ok(Tagged { version: found.row.updated_at, data: found.into() })
}

/// A new status for a submission.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateSubmissionRequest {
    /// `read`, `archived`, `spam`, or `new` (e.g. not spam after all).
    status: SubmissionStatus,
}

/// Change the status of a submission.
///
/// Requires `forms.review`. `If-Match` is required. Moving a submission out of `spam` sends
/// the notification mail as for a new one (`forms.submission_received`).
#[utoipa::path(
    patch,
    path = "/submissions/{id}",
    extensions(("x-permissions" = json!(["forms.review"]))),
    operation_id = "update_form_submission",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the submission."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = UpdateSubmissionRequest,
    responses(
        (status = 200, description = "The submission after the change.", body = Data<SubmissionResponse>, headers(("ETag" = String, description = "New version of the submission."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.review`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No submission has this id; code `forms.submission_not_found`.", body = Problem),
        (status = 412, description = "The submission changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The status is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_submission(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateSubmissionRequest>,
) -> Result<Tagged<SubmissionResponse>, Error> {
    let expected = if_match.required()?;
    let found =
        service.set_submission_status(&tenant, &principal, id, input.status, expected).await?;
    Ok(Tagged { version: found.row.updated_at, data: found.into() })
}

/// Delete a submission.
///
/// Requires `forms.review`.
#[utoipa::path(
    delete,
    path = "/submissions/{id}",
    extensions(("x-permissions" = json!(["forms.review"]))),
    operation_id = "delete_form_submission",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the submission.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.review`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No submission has this id; code `forms.submission_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_submission(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_submission(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Whose data.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EmailQuery {
    /// The email address of the person (any case).
    #[param(example = "ada@example.com")]
    email: Option<String>,
}

impl EmailQuery {
    fn email(&self) -> Result<&str, Error> {
        self.email.as_deref().ok_or_else(|| {
            Error::validation(vec![FieldError::new("email", "required", "is required")])
        })
    }
}

/// Everything the forms hold about one email address.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct EmailDataResponse {
    /// The address, as matched (lower case).
    email: String,
    /// Every submission with this address, oldest first, any status.
    submissions: Vec<SubmissionResponse>,
}

/// Result of erasing an email address.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ErasedResponse {
    /// Submissions deleted.
    deleted: u64,
}

/// Export the data of an email address.
///
/// Requires `forms.export`. For a request of the person the data is about (someone who sent
/// forms, without an account): every submission sent with this address, in every form.
#[utoipa::path(
    get,
    path = "/data",
    extensions(("x-permissions" = json!(["forms.export"]))),
    operation_id = "export_forms_email_data",
    security(("bearer" = [])),
    params(EmailQuery),
    responses(
        (status = 200, description = "The data.", body = Data<EmailDataResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.export`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`email` is missing or not an email address; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_email_data(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<EmailQuery>,
) -> Result<Data<EmailDataResponse>, Error> {
    // Kiểm quyền trước khi đọc `email`: người không có quyền nhận 403, không phải 422 (D-245).
    principal.require(&crate::permissions::FORMS_EXPORT)?;
    let found = service.submissions_of_email(&tenant, &principal, query.email()?).await?;
    let email = autoxide_kernel::email::normalize(query.email()?);
    let submissions = found.into_iter().map(SubmissionResponse::from).collect();
    Ok(Data { data: EmailDataResponse { email, submissions } })
}

/// Erase the data of an email address.
///
/// Requires `forms.export`. Deletes for good every submission sent with this address, in
/// every form (a request of the person the data is about). The event
/// `forms.submissions_erased` does not carry the address.
#[utoipa::path(
    delete,
    path = "/data",
    extensions(("x-permissions" = json!(["forms.export"]))),
    operation_id = "erase_forms_email_data",
    security(("bearer" = [])),
    params(EmailQuery),
    responses(
        (status = 200, description = "How many submissions were deleted.", body = Data<ErasedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.export`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`email` is missing or not an email address; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn delete_email_data(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<EmailQuery>,
) -> Result<Data<ErasedResponse>, Error> {
    // Kiểm quyền trước khi đọc `email`: người không có quyền nhận 403, không phải 422 (D-245).
    principal.require(&crate::permissions::FORMS_EXPORT)?;
    let deleted = service.erase_email(&tenant, &principal, query.email()?).await?;
    Ok(Data { data: ErasedResponse { deleted } })
}

/// Tối đa số bài chọn trong một lần xuất theo `ids`.
const MAX_PICKED: usize = 200;

/// `ids` của bản xuất: UUID cách nhau bằng dấu phẩy, bỏ trùng; sai dạng hay quá nhiều thì thêm lỗi.
fn picked_ids(raw: Option<&str>, problems: &mut Vec<FieldError>) -> Vec<Uuid> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|part| !part.is_empty()) {
        match part.parse::<Uuid>() {
            Ok(id) if !ids.contains(&id) => ids.push(id),
            Ok(_) => {}
            Err(_) => {
                problems.push(FieldError::new(
                    "ids",
                    "invalid",
                    "must be submission ids separated by commas",
                ));
                return Vec::new();
            }
        }
    }
    if ids.len() > MAX_PICKED {
        problems.push(
            FieldError::new("ids", "too_many", "at most {max} ids").with_param("max", MAX_PICKED),
        );
    }
    ids
}
