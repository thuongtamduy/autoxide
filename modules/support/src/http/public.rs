//! Endpoint công khai: mở phiếu từ website (đăng nhập hay vãng lai), và khách vãng lai xem / viết /
//! xác thực bằng token phiếu (`token`). Không bao giờ trả ghi chú nội bộ; email và số điện thoại đã
//! che.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::views::{MessageResponse, OpenedTicket, TicketDetail, TicketResponse};
use crate::guest::PublicOpening;
use crate::messages::NewMessage;
use crate::service::{SupportService, Viewer};

/// A ticket from the website. Signed-in customers send only the question; guests also send
/// exactly one of `email` or `phone`, and the anti-spam fields of the form.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OpenSupportTicketRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenTicketRequest {
    /// Shop or marketplace store that should answer; omitted: the root unit.
    org_id: Option<Uuid>,
    /// Your name.
    name: Option<String>,
    /// Guests: where we answer (a code to confirm it is sent there).
    email: Option<String>,
    /// Guests: a mobile number, instead of `email` (a code is sent by SMS).
    phone: Option<String>,
    /// A ticket category of the shop (default: `order`, `product`, `payment`, `account`, `other`).
    #[schema(example = "order")]
    category: String,
    subject: String,
    /// Your question, plain text.
    body: String,
    /// Language for our messages to you (`vi`, `en`).
    locale: Option<String>,
    /// An earlier ticket of yours this one follows up (closed tickets do not reopen).
    follow_up_of: Option<Uuid>,
    /// Signed-in customers: media items you uploaded (at most 5). Guests cannot attach files.
    #[serde(default)]
    media_ids: Vec<Uuid>,
    /// Guests: the token from `GET /api/v1/antispam/token?scope=support:tickets`.
    form_token: Option<String>,
    /// Guests: the hidden trap field of the form; leave it empty.
    trap: Option<String>,
    /// Guests: the answer to the challenge, when the token came with one.
    challenge: Option<String>,
}

/// Open a support ticket from the website.
///
/// Public. Signed in: the ticket belongs to your account (see `/me/tickets`). Guest: give `email`
/// or `phone`; the answer carries `access_token` once — keep it to see and answer the ticket — and
/// a code is sent to confirm your contact, needed before your second message. Guests are limited
/// per network address, email and phone number. Sending the same `form_token` again returns the
/// same ticket.
#[utoipa::path(
    post,
    path = "/public/tickets",
    extensions(("x-public" = json!(true))),
    operation_id = "open_support_ticket",
    security((), ("bearer" = [])),
    request_body = OpenTicketRequest,
    responses(
        (status = 201, description = "Opened.", body = Data<OpenedTicket>),
        (status = 400, description = "The form token or challenge is missing, invalid or expired (codes `antispam.token_missing`, `antispam.token_invalid`, `antispam.token_expired`, `antispam.challenge_required`, `antispam.challenge_failed`): reload the form.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), the category is unknown, or a file was not uploaded by you (`support.attachment_not_owned`).", body = Problem),
        (status = 429, description = "Too many tickets from you; codes `antispam.rate_limited`, `support.too_many_requests`.", body = Problem),
    )
)]
pub(crate) async fn open_ticket(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    client: ClientInfo,
    ValidJson(input): ValidJson<OpenTicketRequest>,
) -> Result<Created<OpenedTicket>, Error> {
    let opening = PublicOpening {
        org_id: input.org_id,
        name: input.name,
        email: input.email,
        phone: input.phone,
        category: input.category,
        subject: input.subject,
        body: input.body,
        locale: input.locale,
        follow_up_of: input.follow_up_of,
        media_ids: input.media_ids,
        form_token: input.form_token,
        trap: input.trap,
        challenge: input.challenge,
    };
    let opened = service.open_public(&tenant, principal.as_ref(), &client, opening).await?;
    let id = opened.ticket.id;
    Ok(Created::new(
        format!("/support/public/tickets/{id}"),
        OpenedTicket {
            ticket: TicketResponse::new(opened.ticket, false),
            access_token: opened.access_token,
        },
    ))
}

/// The ticket token of a guest.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TokenQuery {
    /// The `access_token` received when the ticket was opened.
    token: String,
}

/// See a guest ticket.
///
/// Public, with the ticket token. Internal notes are never shown; the token opens this ticket
/// only, and stops working some days after the ticket is closed.
#[utoipa::path(
    get,
    path = "/public/tickets/{id}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_support_ticket",
    security(()),
    params(("id" = Uuid, Path, description = "Id of the ticket."), TokenQuery),
    responses(
        (status = 200, description = "The ticket.", body = Data<TicketDetail>),
        (status = 404, description = "No ticket with this id and token; code `support.ticket_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_public_ticket(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    Query(query): Query<TokenQuery>,
) -> Result<Data<TicketDetail>, Error> {
    let ticket = service.by_token(&tenant, id, &query.token).await?;
    let messages = service.messages(&tenant, Viewer::Guest, &ticket).await?;
    Ok(Data { data: TicketDetail::new(ticket, messages, false) })
}

/// A message from a guest.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = PublicSupportMessageRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicMessageRequest {
    /// The ticket token.
    token: String,
    /// Plain text.
    body: String,
}

/// Write on a guest ticket.
///
/// Public, with the ticket token. Confirm your contact first (`/verify`), otherwise
/// `support.verification_required`. A solved ticket opens again; a closed ticket cannot take
/// messages.
#[utoipa::path(
    post,
    path = "/public/tickets/{id}/messages",
    extensions(("x-public" = json!(true))),
    operation_id = "add_public_support_message",
    security(()),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = PublicMessageRequest,
    responses(
        (status = 201, description = "Added.", body = Data<MessageResponse>),
        (status = 403, description = "The contact is not confirmed yet; code `support.verification_required`.", body = Problem),
        (status = 404, description = "No ticket with this id and token; code `support.ticket_not_found`.", body = Problem),
        (status = 409, description = "The ticket is closed; code `support.ticket_closed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn add_public_message(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<PublicMessageRequest>,
) -> Result<Created<MessageResponse>, Error> {
    service.by_token(&tenant, id, &input.token).await?;
    let new = NewMessage { body: input.body, ..NewMessage::default() };
    let (_, message) = service.add_message(&tenant, Viewer::Guest, id, new).await?;
    Ok(Created::new(format!("/support/public/tickets/{id}"), message.into()))
}

/// Confirm the contact of a guest ticket.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = VerifySupportTicketRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerifyRequest {
    /// The ticket token.
    token: String,
    /// The code we sent. Omit it to get a new code.
    code: Option<String>,
}

/// Confirm the email or phone number of a guest ticket.
///
/// Public, with the ticket token. Without `code`: sends a new code (a few times per hour). With
/// `code`: checks it (5 tries per code, valid 15 minutes); the answer shows `requester.verified`.
#[utoipa::path(
    post,
    path = "/public/tickets/{id}/verify",
    extensions(("x-public" = json!(true))),
    operation_id = "verify_support_ticket",
    security(()),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = VerifyRequest,
    responses(
        (status = 200, description = "The ticket; `requester.verified` tells whether the contact is confirmed.", body = Data<TicketResponse>),
        (status = 404, description = "No ticket with this id and token; code `support.ticket_not_found`.", body = Problem),
        (status = 422, description = "The code is wrong or expired; see `errors`.", body = Problem),
        (status = 429, description = "Too many codes asked; code `support.too_many_requests`.", body = Problem),
        (status = 503, description = "Codes cannot be sent on this server; code `support.verification_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn verify_ticket(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<VerifyRequest>,
) -> Result<Data<TicketResponse>, Error> {
    let ticket = service.verify(&tenant, id, &input.token, input.code.as_deref()).await?;
    Ok(Data { data: TicketResponse::new(ticket, false) })
}
