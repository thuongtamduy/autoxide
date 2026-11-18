//! Endpoint của người mua có tài khoản: phiếu của chính mình. Tài khoản người mua (`customer`) không
//! có permission (bước 34): chỉ cần đăng nhập, và chỉ chạm phiếu có `requester_user_id` là mình.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::views::{MessageResponse, TicketDetail, TicketResponse};
use crate::messages::NewMessage;
use crate::service::{SupportService, Viewer};

/// List your own support tickets.
///
/// Any signed-in account (customer accounts too). Newest first unless `order=asc`. Internal notes
/// are never shown.
#[utoipa::path(
    get,
    path = "/me/tickets",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_support_tickets",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your tickets.", body = Page<TicketResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_my_tickets(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<TicketResponse>, Error> {
    let page =
        service.list_mine(&tenant, principal.user_id(), &PageRequest::try_from(page)?).await?;
    Ok(page.map(|ticket| TicketResponse::new(ticket, false)))
}

/// Get one of your support tickets with its messages.
///
/// Any signed-in account; a ticket of someone else answers 404.
#[utoipa::path(
    get,
    path = "/me/tickets/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_support_ticket",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    responses(
        (status = 200, description = "The ticket.", body = Data<TicketDetail>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no ticket with this id; code `support.ticket_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_my_ticket(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TicketDetail>, Error> {
    let viewer = Viewer::Customer(principal.user_id());
    let ticket = service.get(&tenant, viewer, id).await?;
    let messages = service.messages(&tenant, viewer, &ticket).await?;
    Ok(Data { data: TicketDetail::new(ticket, messages, false) })
}

/// A message from the customer.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = MySupportMessageRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct MyMessageRequest {
    /// Plain text.
    body: String,
    /// Media items you uploaded yourself (at most 5).
    #[serde(default)]
    media_ids: Vec<Uuid>,
}

/// Write on one of your support tickets.
///
/// Any signed-in account. A solved ticket opens again (`pending_agent`); a closed ticket cannot
/// take messages — open a new ticket with `follow_up_of`. Retry safely with an `Idempotency-Key`.
#[utoipa::path(
    post,
    path = "/me/tickets/{id}/messages",
    extensions(("x-permissions" = json!([]))),
    operation_id = "add_my_support_message",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = MyMessageRequest,
    responses(
        (status = 201, description = "Added.", body = Data<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no ticket with this id; code `support.ticket_not_found`.", body = Problem),
        (status = 409, description = "The ticket is closed; code `support.ticket_closed`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or a file was not uploaded by you (`support.attachment_not_owned`).", body = Problem),
    )
)]
pub(crate) async fn add_my_message(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MyMessageRequest>,
) -> Result<Created<MessageResponse>, Error> {
    let new = NewMessage { body: input.body, media_ids: input.media_ids, ..NewMessage::default() };
    let viewer = Viewer::Customer(principal.user_id());
    let (_, message) = service.add_message(&tenant, viewer, id, new).await?;
    Ok(Created::new(format!("/support/me/tickets/{id}"), message.into()))
}

/// Proof that you own guest tickets: one guest ticket you verified, with its token.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ClaimSupportTicketsRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimRequest {
    /// A guest ticket whose email or phone number you verified.
    ticket_id: Option<Uuid>,
    /// Its `access_token`.
    token: Option<String>,
}

/// How many guest tickets were attached.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ClaimSupportTicketsResponse)]
pub(crate) struct ClaimResponse {
    pub claimed: u64,
}

/// Attach your guest tickets to your account.
///
/// Any signed-in account. Attaches verified guest tickets with the phone number your account
/// verified, and — when you send a verified guest ticket and its token — every verified guest
/// ticket with the same email or phone number. Calling again changes nothing more.
#[utoipa::path(
    post,
    path = "/me/claim",
    extensions(("x-permissions" = json!([]))),
    operation_id = "claim_support_tickets",
    security(("bearer" = [])),
    request_body = ClaimRequest,
    responses(
        (status = 200, description = "Tickets attached now.", body = Data<ClaimResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The ticket you sent is not verified yet; code `support.verification_required`.", body = Problem),
        (status = 404, description = "The ticket or token is wrong; code `support.ticket_not_found`.", body = Problem),
    )
)]
pub(crate) async fn claim_tickets(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ClaimRequest>,
) -> Result<Data<ClaimResponse>, Error> {
    let proof = input.ticket_id.zip(input.token);
    let claimed = service.claim(&tenant, &principal, proof).await?;
    Ok(Data { data: ClaimResponse { claimed } })
}
