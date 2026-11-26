//! Endpoint của nhân viên: danh sách, một phiếu, đổi phiếu, trả lời, ghi chú nội bộ, ẩn tin, mở phiếu
//! thay người mua (ghi hộ cuộc gọi).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::views::{MessageResponse, OpenedTicket, TicketDetail, TicketResponse};
use crate::messages::NewMessage;
use crate::permissions::{TICKETS_MANAGE, TICKETS_READ, TICKETS_REPLY};
use crate::service::{SupportService, Viewer};
use crate::status::{Channel, Priority, Resolution, Status};
use crate::tickets::{Opening, TicketChanges, TicketFilter, Who};

/// Filters of the ticket list; all optional and combined with AND.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TicketQuery {
    /// Only tickets in this status.
    status: Option<Status>,
    /// `me`, `none` or a user id.
    assignee: Option<String>,
    /// Only tickets assigned to this team.
    team_id: Option<Uuid>,
    /// Only tickets of this unit (not the units below it).
    org_id: Option<Uuid>,
    /// Only tickets of this category (its key).
    category: Option<String>,
    /// Only tickets of this priority.
    priority: Option<Priority>,
    /// Only tickets that missed (`true`) or kept (`false`) their response-time target.
    breached: Option<bool>,
    /// Only tickets linked to this record, as `<kind>:<id>` (e.g. `orders.order:<id>`).
    link: Option<String>,
    /// Text in the number or subject; accents do not matter.
    q: Option<String>,
}

impl TicketQuery {
    fn filter(self, principal: &Principal) -> Result<TicketFilter, Error> {
        let assignee = match self.assignee.as_deref() {
            None => None,
            Some("me") => Some(Some(principal.user_id())),
            Some("none") => Some(None),
            Some(raw) => Some(Some(raw.parse::<Uuid>().map_err(|_| {
                Error::validation(vec![FieldError::new(
                    "assignee",
                    "invalid",
                    "must be `me`, `none` or a user id",
                )])
            })?)),
        };
        Ok(TicketFilter {
            status: self.status,
            assignee,
            team_id: self.team_id,
            org_id: self.org_id,
            category: self.category,
            priority: self.priority,
            breached: self.breached,
            link: self.link,
            q: self.q,
        })
    }
}

/// List the support tickets in your scope.
///
/// Requires `support.tickets.read`; its scope decides which units' tickets you see (`unit`,
/// `subtree`, `tenant`; `own`: tickets assigned to you). Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/tickets",
    extensions(("x-permissions" = json!(["support.tickets.read"]))),
    operation_id = "list_support_tickets",
    security(("bearer" = [])),
    params(TicketQuery, PageQuery),
    responses(
        (status = 200, description = "One page of tickets.", body = Page<TicketResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_tickets(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<TicketQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<TicketResponse>, Error> {
    principal.require(&TICKETS_READ)?;
    let filter = query.filter(&principal)?;
    let page = service.list(&tenant, &principal, &filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(|ticket| TicketResponse::new(ticket, true)))
}

/// Who asked, for a ticket staff open on behalf of a customer: exactly one of the three.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SupportRequesterInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequesterInput {
    /// A customer account.
    user_id: Option<Uuid>,
    email: Option<String>,
    phone: Option<String>,
    /// Name to show.
    #[serde(default)]
    name: String,
}

/// A ticket staff open for a customer (a phone call, a visit…).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = CreateSupportTicketRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateTicketRequest {
    /// Unit of the ticket; you need `support.tickets.reply` there. Omitted: the root unit.
    org_id: Option<Uuid>,
    #[validate(nested)]
    requester: RequesterInput,
    /// A ticket category of the shop (default: `order`, `product`, `payment`, `account`, `other`).
    #[schema(example = "order")]
    category: String,
    subject: String,
    /// What the customer asked.
    body: String,
    priority: Option<Priority>,
    /// Attached media items.
    #[serde(default)]
    media_ids: Vec<Uuid>,
}

/// Open a ticket for a customer.
///
/// Requires `support.tickets.reply` at the unit of the ticket. The contact counts as verified.
/// Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/tickets",
    extensions(("x-permissions" = json!(["support.tickets.reply"]))),
    operation_id = "create_support_ticket",
    security(("bearer" = [])),
    request_body = CreateTicketRequest,
    responses(
        (status = 201, description = "Opened.", body = Data<OpenedTicket>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), the category is unknown, or a file is not in the media library (`support.attachment_not_owned`).", body = Problem),
    )
)]
pub(crate) async fn create_ticket(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateTicketRequest>,
) -> Result<Created<OpenedTicket>, Error> {
    principal.require(&TICKETS_REPLY)?;
    let requester = match (input.requester.user_id, input.requester.email, input.requester.phone) {
        (Some(user), None, None) => Who::User(user),
        (None, Some(email), None) if autoxide_kernel::email::is_valid(email.trim()) => {
            Who::Email(autoxide_kernel::email::normalize(&email))
        }
        (None, None, Some(raw)) => match PhoneNumber::parse(&raw, "84") {
            Some(phone) => Who::Phone(phone.as_str().to_owned()),
            None => return Err(requester_invalid()),
        },
        _ => return Err(requester_invalid()),
    };
    let org = input.org_id.unwrap_or_else(|| tenant.id().as_uuid());
    let opening = Opening {
        org_id: Some(org),
        requester,
        requester_name: input.requester.name,
        verified: true,
        category: input.category,
        subject: input.subject,
        body: input.body,
        priority: input.priority,
        channel: Channel::Staff,
        media_ids: input.media_ids,
        links: Vec::new(),
        reference: None,
        follow_up_of: None,
        locale: None,
    };
    let opened = service.open_by_staff(&tenant, &principal, opening).await?;
    let id = opened.ticket.id;
    Ok(Created::new(
        format!("/support/tickets/{id}"),
        OpenedTicket { ticket: TicketResponse::new(opened.ticket, true), access_token: None },
    ))
}

fn requester_invalid() -> Error {
    Error::validation(vec![FieldError::new(
        "requester",
        "invalid",
        "give exactly one of `user_id`, a valid `email` or a valid `phone`",
    )])
}

/// Get a support ticket with all its messages and internal notes.
///
/// Requires `support.tickets.read` covering the ticket's unit; a ticket outside your scope answers
/// 404.
#[utoipa::path(
    get,
    path = "/tickets/{id}",
    extensions(("x-permissions" = json!(["support.tickets.read"]))),
    operation_id = "get_support_ticket",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    responses(
        (status = 200, description = "The ticket.", body = Data<TicketDetail>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No ticket with this id in your scope; code `support.ticket_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_ticket(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<TicketDetail>, Error> {
    principal.require(&TICKETS_READ)?;
    let viewer = Viewer::Staff(&principal);
    let ticket = service.get(&tenant, viewer, id).await?;
    let messages = service.messages(&tenant, viewer, &ticket).await?;
    let links = service.links(&tenant, ticket.id).await?;
    let mut data = TicketDetail::new(ticket.clone(), messages, true);
    data.links = links;
    Ok(Tagged { version: ticket.updated_at, data })
}

/// Changes to a ticket. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = UpdateSupportTicketRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateTicketRequest {
    /// New status; only steps of the ticket workflow are allowed (`support.invalid_transition`).
    status: Option<Status>,
    /// How it ended, with `status` `solved` or `closed` (default `answered`).
    resolution: Option<Resolution>,
    priority: Option<Priority>,
    category: Option<String>,
    /// Assign to this user (yourself, or a member of a support team of the unit); `null`: nobody.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Uuid>)]
    assignee_user_id: Option<Option<Uuid>>,
    /// Assign to this support team of the unit; `null`: no team.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<Uuid>)]
    team_id: Option<Option<Uuid>>,
}

/// Phân biệt "không gửi trường" (`None`) với "gửi `null`" (`Some(None)`).
fn double_option<'de, D>(deserializer: D) -> Result<Option<Option<Uuid>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Uuid>::deserialize(deserializer).map(Some)
}

/// Change status, priority, category or assignee of a ticket.
///
/// Requires `support.tickets.reply` covering the ticket; its unit must be active. Send the `ETag`
/// you read in `If-Match`: when two people change the ticket at once, exactly one wins and the
/// other gets 412.
#[utoipa::path(
    patch,
    path = "/tickets/{id}",
    extensions(("x-permissions" = json!(["support.tickets.reply"]))),
    operation_id = "update_support_ticket",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the ticket since; otherwise `412`.")),
    request_body = UpdateTicketRequest,
    responses(
        (status = 200, description = "The ticket after the change.", body = Data<TicketResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply` (`permission_denied`), it does not cover this ticket (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No ticket with this id in your scope; code `support.ticket_not_found`.", body = Problem),
        (status = 409, description = "The ticket is closed (`support.ticket_closed`) or the status step is not allowed (`support.invalid_transition`).", body = Problem),
        (status = 412, description = "`If-Match` does not match: the ticket changed since you read it. Get it again and retry; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or the assignee does not work on this unit (`support.assignee_out_of_scope`).", body = Problem),
    )
)]
pub(crate) async fn update_ticket(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateTicketRequest>,
) -> Result<Tagged<TicketResponse>, Error> {
    principal.require(&TICKETS_REPLY)?;
    let changes = TicketChanges {
        status: input.status,
        resolution: input.resolution,
        priority: input.priority,
        category: input.category,
        assignee_user_id: input.assignee_user_id,
        team_id: input.team_id,
    };
    let expected = if_match.expected()?;
    let ticket = service.update(&tenant, &principal, id, changes, expected).await?;
    Ok(Tagged { version: ticket.updated_at, data: TicketResponse::new(ticket, true) })
}

/// A message to add.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = AddSupportMessageRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddMessageRequest {
    /// Plain text.
    body: String,
    /// `true`: an internal note only staff see.
    #[serde(default)]
    internal: bool,
    /// Attached media items (at most 5).
    #[serde(default)]
    media_ids: Vec<Uuid>,
    /// Colleagues to notify, in an internal note (at most 20).
    #[serde(default)]
    mentions: Vec<Uuid>,
    /// The text comes from an AI suggestion (shown with a label).
    #[serde(default)]
    ai_assisted: bool,
}

/// Answer a ticket, or add an internal note.
///
/// Requires `support.tickets.reply` covering the ticket. A public answer moves the ticket to
/// `pending_customer` (the response-time clock stops) and notifies the customer; an internal note
/// (`internal: true`) notifies the mentioned colleagues. Retry safely with an `Idempotency-Key`.
#[utoipa::path(
    post,
    path = "/tickets/{id}/messages",
    extensions(("x-permissions" = json!(["support.tickets.reply"]))),
    operation_id = "add_support_message",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = AddMessageRequest,
    responses(
        (status = 201, description = "Added.", body = Data<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply` (`permission_denied`), it does not cover this ticket (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No ticket with this id in your scope; code `support.ticket_not_found`.", body = Problem),
        (status = 409, description = "The ticket is closed; code `support.ticket_closed`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or a file is not in the media library (`support.attachment_not_owned`).", body = Problem),
    )
)]
pub(crate) async fn add_message(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AddMessageRequest>,
) -> Result<Created<MessageResponse>, Error> {
    principal.require(&TICKETS_REPLY)?;
    let new = NewMessage {
        body: input.body,
        internal: input.internal,
        media_ids: input.media_ids,
        mentions: input.mentions,
        ai_assisted: input.ai_assisted,
    };
    let (_, message) = service.add_message(&tenant, Viewer::Staff(&principal), id, new).await?;
    Ok(Created::new(format!("/support/tickets/{id}"), message.into()))
}

/// Why a message is hidden.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = HideSupportMessageRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct HideMessageRequest {
    /// Kept in the audit log; 1 to 500 characters.
    reason: String,
}

/// Hide a message of a ticket.
///
/// Requires `support.tickets.manage` covering the ticket. Messages are never edited or deleted: a
/// mistaken message is hidden with a reason; the customer sees that a message was hidden.
#[utoipa::path(
    post,
    path = "/tickets/{id}/messages/{message_id}/hide",
    extensions(("x-permissions" = json!(["support.tickets.manage"]))),
    operation_id = "hide_support_message",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket."), ("message_id" = Uuid, Path, description = "Id of the message.")),
    request_body = HideMessageRequest,
    responses(
        (status = 200, description = "The message, now hidden.", body = Data<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.manage` (`permission_denied`), it does not cover this ticket (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No such ticket in your scope (`support.ticket_not_found`) or message (`support.message_not_found`).", body = Problem),
        (status = 422, description = "The reason is missing or too long; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn hide_message(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, message_id)): Path<(Uuid, Uuid)>,
    ValidJson(input): ValidJson<HideMessageRequest>,
) -> Result<Data<MessageResponse>, Error> {
    principal.require(&TICKETS_MANAGE)?;
    let message = service.hide_message(&tenant, &principal, id, message_id, &input.reason).await?;
    Ok(Data { data: message.into() })
}
