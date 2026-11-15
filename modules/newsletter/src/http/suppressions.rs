//! Endpoint danh sách chặn (`/suppressions…`) và báo bounce (`/bounces`). Đọc cần
//! `newsletter.read`; ghi cần `newsletter.manage`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::page_request;
use crate::audience::bounces::{BounceKind, BounceOutcome, SuppressionFilter};
use crate::entity::suppression;
use crate::service::NewsletterService;

/// An address that gets no campaign mail.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SuppressionResponse {
    pub id: Uuid,
    #[schema(example = "ada@example.com")]
    pub email: String,
    /// `hard_bounce`, `soft_bounce`, `complaint` or `manual`.
    pub reason: String,
    /// A note, or the SMTP code of a bounce.
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<suppression::Model> for SuppressionResponse {
    fn from(row: suppression::Model) -> Self {
        SuppressionResponse {
            id: row.id,
            email: row.email,
            reason: row.reason,
            note: row.note,
            created_at: row.created_at,
        }
    }
}

/// Which suppressions to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SuppressionsQuery {
    /// Part of the email address, any case.
    pub email: Option<String>,
}

/// An address to suppress by hand.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateSuppressionRequest {
    #[schema(example = "ada@example.com")]
    email: String,
    /// Up to 255 characters.
    note: Option<String>,
}

/// A bounce or complaint reported by a mail service.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BounceRequest {
    /// The address that bounced.
    #[schema(example = "ada@example.com")]
    email: String,
    #[serde(rename = "type")]
    kind: BounceKind,
    /// What the mail service said (kept as the note of a suppression), up to 255 characters.
    reason: Option<String>,
}

/// List the suppressed addresses.
///
/// Requires `newsletter.read`. Newest first; follow `pagination.next_cursor` for more.
#[utoipa::path(
    get,
    path = "/suppressions",
    extensions(("x-permissions" = json!(["newsletter.read"]))),
    operation_id = "list_newsletter_suppressions",
    security(("bearer" = [])),
    params(SuppressionsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of suppressions.", body = Page<SuppressionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_suppressions(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<SuppressionsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<SuppressionResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    let filter = SuppressionFilter { email: query.email };
    Ok(service.list_suppressions(&tenant, &principal, &filter, &page).await?.map(Into::into))
}

/// Suppress an address.
///
/// Requires `newsletter.manage`. The address gets no campaign mail from now on, whatever its
/// subscriptions. Suppressing an address already suppressed returns the existing entry.
#[utoipa::path(
    post,
    path = "/suppressions",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "create_newsletter_suppression",
    security(("bearer" = [])),
    request_body = CreateSuppressionRequest,
    responses(
        (status = 201, description = "Suppressed.", body = Data<SuppressionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_suppression(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateSuppressionRequest>,
) -> Result<Created<SuppressionResponse>, Error> {
    let row =
        service.add_suppression(&tenant, &principal, &input.email, input.note.as_deref()).await?;
    Ok(Created::new(format!("/newsletter/suppressions/{}", row.id), row.into()))
}

/// Take an address off the suppression list.
///
/// Requires `newsletter.manage`. Its soft-bounce count starts again from zero.
#[utoipa::path(
    delete,
    path = "/suppressions/{id}",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "delete_newsletter_suppression",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the suppression.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No suppression has this id; code `newsletter.suppression_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_suppression(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.remove_suppression(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Report a bounce or complaint.
///
/// Requires `newsletter.manage` (give a mail service an API key with it). `hard` and
/// `complaint` suppress the address at once; `soft` suppresses it after
/// `AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT` reports. Permanent SMTP failures of mails sent by
/// this server are counted as hard bounces without calling this endpoint.
#[utoipa::path(
    post,
    path = "/bounces",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "report_newsletter_bounce",
    security(("bearer" = [])),
    request_body = BounceRequest,
    responses(
        (status = 200, description = "Whether the address is now suppressed.", body = Data<BounceOutcome>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn report_bounce(
    State(service): State<Arc<NewsletterService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<BounceRequest>,
) -> Result<Data<BounceOutcome>, Error> {
    let outcome = service
        .report_bounce(&tenant, &principal, &input.email, input.kind, input.reason.as_deref())
        .await?;
    Ok(Data { data: outcome })
}
