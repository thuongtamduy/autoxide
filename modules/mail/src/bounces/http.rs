//! Endpoint sổ thư bị dội, dưới `/api/v1/mail/bounces`. Handler chỉ lấy dữ liệu vào, gọi service, trả envelope;
//! permission do service kiểm.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::{Bounce, BounceFilter, BounceStats, Bounces};

pub(crate) fn routes(service: Arc<Bounces>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_mail_bounces))
        .routes(routes!(get_mail_bounce_stats))
        .routes(routes!(get_mail_bounce, delete_mail_bounce))
        .routes(routes!(release_mail_bounce))
        .with_state(service)
}

/// A mail address of this tenant that bounced: how often, why, and whether mail to it is
/// suppressed.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct BounceResponse {
    id: Uuid,
    /// The address, in lower case.
    #[schema(example = "ada@example.com")]
    email: String,
    /// Hard bounces since the record was created: the receiving server refused the address
    /// itself (SMTP enhanced status 5.1.x or 5.2.x).
    hard_bounces: u32,
    /// Hard bounces counted towards suppression; back to 0 when the address is released.
    strikes: u32,
    /// SMTP status of the last bounce, e.g. `550 5.1.1`.
    #[schema(example = "550 5.1.1")]
    last_code: String,
    first_bounced_at: Timestamp,
    last_bounced_at: Timestamp,
    /// Whether mail to this address is suppressed: new mail to it is not sent.
    suppressed: bool,
    suppressed_at: Option<Timestamp>,
    /// Mails not sent because the address was suppressed.
    skipped: u32,
    last_skipped_at: Option<Timestamp>,
    /// What the last skipped mail was, from the sender's key, e.g. `auth.password_reset`.
    #[schema(example = "auth.password_reset")]
    last_skipped_source: Option<String>,
    released_at: Option<Timestamp>,
    /// The user who last took the address off the suppression list.
    released_by: Option<Uuid>,
}

fn count(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

impl From<Bounce> for BounceResponse {
    fn from(bounce: Bounce) -> Self {
        let row = bounce.row;
        BounceResponse {
            id: row.id,
            email: row.address,
            hard_bounces: count(row.hard_count),
            strikes: count(row.strikes),
            last_code: row.last_code,
            first_bounced_at: row.first_at,
            last_bounced_at: row.last_at,
            suppressed: row.suppressed_at.is_some(),
            suppressed_at: row.suppressed_at,
            skipped: count(row.skipped_count),
            last_skipped_at: row.last_skipped_at,
            last_skipped_source: row.last_skipped_source,
            released_at: row.released_at,
            released_by: row.released_by,
        }
    }
}

/// Filters of the bounce list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BounceQuery {
    /// Only this address (any case; `Name <address>` is accepted).
    email: Option<String>,
    /// `true`: only suppressed addresses; `false`: only those still mailed.
    suppressed: Option<bool>,
}

/// Bounce figures of this tenant.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct BounceStatsResponse {
    /// Addresses with a bounce record.
    addresses: u64,
    /// Addresses whose mail is suppressed.
    suppressed: u64,
    /// Addresses that bounced in the last 30 days.
    bounced_last_30_days: u64,
    /// Addresses that had a mail skipped (suppressed) in the last 30 days.
    skipped_last_30_days: u64,
}

impl From<BounceStats> for BounceStatsResponse {
    fn from(stats: BounceStats) -> Self {
        BounceStatsResponse {
            addresses: stats.addresses,
            suppressed: stats.suppressed,
            bounced_last_30_days: stats.bounced_last_30_days,
            skipped_last_30_days: stats.skipped_last_30_days,
        }
    }
}

/// List the mail addresses of this tenant that bounced.
///
/// Requires `mail.bounces.read`. Answers "did mail to this address bounce, and why":
/// `?email=ada@example.com`. An address that hard-bounced `AUTOXIDE_MAIL_HARD_BOUNCE_LIMIT` times
/// is suppressed (`suppressed: true`): new mail to it is not sent, and each skipped mail is
/// counted. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/bounces",
    extensions(("x-permissions" = json!(["mail.bounces.read"]))),
    operation_id = "list_mail_bounces",
    security(("bearer" = [])),
    params(BounceQuery, PageQuery),
    responses(
        (status = 200, description = "One page of bounce records.", body = Page<BounceResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `mail.bounces.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`email` is not a mail address, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_mail_bounces(
    State(service): State<Arc<Bounces>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<BounceQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<BounceResponse>, Error> {
    let filter = BounceFilter { email: filter.email, suppressed: filter.suppressed };
    let page = service.list(&tenant, &principal, &filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(BounceResponse::from))
}

/// Get bounce figures of this tenant.
///
/// Requires `mail.bounces.read`.
#[utoipa::path(
    get,
    path = "/bounces/stats",
    extensions(("x-permissions" = json!(["mail.bounces.read"]))),
    operation_id = "get_mail_bounce_stats",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The figures.", body = Data<BounceStatsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `mail.bounces.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn get_mail_bounce_stats(
    State(service): State<Arc<Bounces>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<BounceStatsResponse>, Error> {
    Ok(Data { data: service.stats(&tenant, &principal).await?.into() })
}

/// Get one bounce record.
///
/// Requires `mail.bounces.read`.
#[utoipa::path(
    get,
    path = "/bounces/{id}",
    extensions(("x-permissions" = json!(["mail.bounces.read"]))),
    operation_id = "get_mail_bounce",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the bounce record.")),
    responses(
        (status = 200, description = "The record.", body = Data<BounceResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `mail.bounces.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such record in this tenant; code `mail.bounce_not_found`.", body = Problem),
    )
)]
async fn get_mail_bounce(
    State(service): State<Arc<Bounces>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BounceResponse>, Error> {
    Ok(Data { data: service.get(&tenant, &principal, id).await?.into() })
}

/// Take an address off the suppression list.
///
/// Requires `mail.bounces.manage`. New mail to the address is sent again; its strikes go back to
/// 0 (the total of hard bounces is kept). Releasing an address that is not suppressed changes
/// nothing. Use it once the address works again (the mailbox was created, the typo fixed).
#[utoipa::path(
    post,
    path = "/bounces/{id}/release",
    extensions(("x-permissions" = json!(["mail.bounces.manage"]))),
    operation_id = "release_mail_bounce",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the bounce record.")),
    responses(
        (status = 200, description = "The record after the release.", body = Data<BounceResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `mail.bounces.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such record in this tenant; code `mail.bounce_not_found`.", body = Problem),
    )
)]
async fn release_mail_bounce(
    State(service): State<Arc<Bounces>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BounceResponse>, Error> {
    Ok(Data { data: service.release(&tenant, &principal, id).await?.into() })
}

/// Delete a bounce record.
///
/// Requires `mail.bounces.manage`. Erases what is kept about the address (a data subject's
/// request); the address is no longer suppressed.
#[utoipa::path(
    delete,
    path = "/bounces/{id}",
    extensions(("x-permissions" = json!(["mail.bounces.manage"]))),
    operation_id = "delete_mail_bounce",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the bounce record.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `mail.bounces.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such record in this tenant; code `mail.bounce_not_found`.", body = Problem),
    )
)]
async fn delete_mail_bounce(
    State(service): State<Arc<Bounces>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
