//! Endpoint xuất CSV lớn bằng job (`submissions/exports.rs`): yêu cầu một bản xuất, xem trạng
//! thái và lấy link tải, tải file qua link ký (không cần đăng nhập).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::{Disposition, content_disposition};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::choice;
use super::inbox::STATUSES;
use crate::service::FormsService;
use crate::submissions::exports::{ExportState, ExportView};
use crate::submissions::inbox::SubmissionStatus;

/// What to export. The body may be omitted.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub(crate) struct CreateExportRequest {
    /// `new`, `read`, `archived` or `spam`; omitted: every status but `spam` (like
    /// `GET /{id}/submissions/export`).
    #[schema(value_type = Option<SubmissionStatus>)]
    pub status: Option<String>,
}

/// A CSV export made in the background.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ExportResponse {
    pub id: Uuid,
    pub form_id: Uuid,
    /// `pending` and `running` until the file is written; then `ready` or `failed`.
    pub state: ExportState,
    /// The `status` the export was asked for; `null`: every status but `spam`.
    pub status: Option<SubmissionStatus>,
    /// Rows in the file (without the header line), once `ready`.
    pub row_count: Option<i64>,
    /// Size of the file in bytes, once `ready`.
    pub size: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// When the file is deleted (`AUTOXIDE_FORMS_EXPORT_KEEP_HOURS` after it was finished).
    pub expires_at: Option<DateTime<Utc>>,
    /// Link to the file, only while `ready`: it needs no sign-in, so it can be opened in a
    /// browser, and it works until `download_expires_at`. An absolute URL when
    /// `AUTOXIDE_HTTP_PUBLIC_URL` is set, otherwise a path on this API. Get the export again
    /// for a fresh link.
    pub download_url: Option<String>,
    pub download_expires_at: Option<DateTime<Utc>>,
}

impl From<ExportView> for ExportResponse {
    fn from(view: ExportView) -> Self {
        let (download_url, download_expires_at) = view.download.unzip();
        ExportResponse {
            id: view.row.id,
            form_id: view.row.form_id,
            state: view.state,
            status: view.row.status_filter.as_deref().and_then(SubmissionStatus::parse),
            row_count: view.row.row_count,
            size: view.row.size,
            created_at: view.row.created_at,
            finished_at: view.row.finished_at,
            expires_at: view.row.expires_at,
            download_url,
            download_expires_at,
        }
    }
}

/// Export the submissions of a form as CSV, in the background.
///
/// Requires `forms.export`. For forms with many submissions: the file is written by a
/// background job, then `GET /api/v1/forms/exports/{export_id}` (the `Location` of the answer)
/// gives a download link. Same content as `GET /{id}/submissions/export`, byte for byte. One
/// export per form at a time. The file holds personal data and is deleted after
/// `AUTOXIDE_FORMS_EXPORT_KEEP_HOURS`, or as soon as submissions of the form are deleted by
/// email or the form is deleted.
#[utoipa::path(
    post,
    path = "/{id}/submissions/exports",
    extensions(("x-permissions" = json!(["forms.export"]))),
    operation_id = "create_form_submissions_export",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the form.")),
    request_body(content = CreateExportRequest, description = "Optional; may be omitted."),
    responses(
        (status = 201, description = "Asked for; the job writes the file shortly.", body = Data<ExportResponse>, headers(("Location" = String, description = "Address of the export."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.export`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No form has this id; code `forms.not_found`.", body = Problem),
        (status = 409, description = "The form already has an export that is not finished; code `forms.export_in_progress` (its id is in the message; follow it with `GET /api/v1/forms/exports/{export_id}`).", body = Problem),
        (status = 422, description = "`status` is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_export(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> Result<Created<ExportResponse>, Error> {
    // Thân không bắt buộc: rỗng thì mọi trạng thái trừ `spam`.
    let input: CreateExportRequest = if body.iter().all(u8::is_ascii_whitespace) {
        CreateExportRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|error| {
            Error::validation(vec![
                FieldError::new("body", "invalid", "is not valid: {reason}")
                    .with_param("reason", error),
            ])
        })?
    };
    let mut problems = Vec::new();
    let status = choice(input.status.as_deref(), "status", STATUSES, &mut problems);
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let view = service.request_export(&tenant, &principal, id, status).await?;
    Ok(Created::new(format!("/forms/exports/{}", view.row.id), view.into()))
}

/// Get a CSV export made in the background.
///
/// Requires `forms.export`. While `pending` or `running`, ask again in a few seconds. Once
/// `ready`, `download_url` is a fresh link to the file each time you ask.
#[utoipa::path(
    get,
    path = "/exports/{export_id}",
    extensions(("x-permissions" = json!(["forms.export"]))),
    operation_id = "get_form_submissions_export",
    security(("bearer" = [])),
    params(("export_id" = Uuid, Path, description = "Id of the export.")),
    responses(
        (status = 200, description = "The export.", body = Data<ExportResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `forms.export`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No export has this id, or it was deleted; code `forms.export_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_export(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(export_id): Path<Uuid>,
) -> Result<Data<ExportResponse>, Error> {
    Ok(Data { data: service.get_export(&tenant, &principal, export_id).await?.into() })
}

/// The signature of a download link.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DownloadQuery {
    /// When the link stops working, in seconds since 1970 (UTC).
    pub expires: i64,
    /// Signature of the link.
    pub signature: String,
}

/// Download the file of a CSV export.
///
/// No sign-in: the link from `download_url` carries its own signature and expiry, so it can be
/// opened in a browser. Do not share it: anyone with the link can download the file until it
/// expires.
#[utoipa::path(
    get,
    path = "/exports/{export_id}/file",
    extensions(("x-public" = json!(true))),
    operation_id = "download_form_submissions_export",
    params(("export_id" = Uuid, Path, description = "Id of the export."), DownloadQuery),
    responses(
        (status = 200, description = "The CSV file.", body = String, content_type = "text/csv", headers(("Content-Disposition" = String, description = "`attachment` with the file name `<key>-submissions.csv`."))),
        (status = 400, description = "`expires` or `signature` is missing or not well formed.", body = Problem),
        (status = 403, description = "The link is wrong or has expired; code `forms.export_link_invalid`. Get a new one from `GET /api/v1/forms/exports/{export_id}`.", body = Problem),
        (status = 404, description = "The export or its file was deleted; code `forms.export_not_found`.", body = Problem),
    )
)]
pub(crate) async fn download_export(
    State(service): State<Arc<FormsService>>,
    tenant: TenantContext,
    Path(export_id): Path<Uuid>,
    Query(query): Query<DownloadQuery>,
) -> Result<Response, Error> {
    let (name, file) =
        service.download_export(&tenant, export_id, query.expires, &query.signature).await?;
    let mut response = Body::from_stream(file.content).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(file.size));
    if let Ok(value) = HeaderValue::from_str(&content_disposition(Disposition::Attachment, &name)) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}
