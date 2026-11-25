//! Liên kết bản ghi, chuyển phiếu, mức hài lòng, câu trả lời mẫu (bước 53 mục 4).

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageRequest, SortedPageQuery, page_of};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::views::TicketResponse;
use crate::extras::{Escalation, SavedReply, SavedReplyInput};
use crate::permissions::{SETTINGS_MANAGE, TICKETS_MANAGE, TICKETS_READ, TICKETS_REPLY};
use crate::service::{SupportService, Viewer};

/// A record to link, as `<module>.<type>:<id>`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = AddSupportLinkRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkRequest {
    #[schema(example = "orders.order:0190a1b2-…")]
    target: String,
}

/// Link a ticket to a record (an order, a product…).
///
/// Requires `support.tickets.reply` covering the ticket. The record must exist (module registry of
/// records). Linking again changes nothing. List tickets of a record with `GET /tickets?link=…`.
#[utoipa::path(
    post,
    path = "/tickets/{id}/links",
    extensions(("x-permissions" = json!(["support.tickets.reply"]))),
    operation_id = "add_support_link",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = LinkRequest,
    responses(
        (status = 200, description = "The links of the ticket.", body = Data<Vec<String>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply` (`permission_denied`), or it does not cover the ticket (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No ticket with this id in your scope; code `support.ticket_not_found`.", body = Problem),
        (status = 422, description = "No such record; code `support.link_not_found`.", body = Problem),
    )
)]
pub(crate) async fn add_link(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<LinkRequest>,
) -> Result<Data<Vec<String>>, Error> {
    principal.require(&TICKETS_REPLY)?;
    Ok(Data { data: service.add_link(&tenant, &principal, id, &input.target).await? })
}

/// Remove a link from a ticket.
///
/// Requires `support.tickets.reply` covering the ticket.
#[utoipa::path(
    delete,
    path = "/tickets/{id}/links/{target}",
    extensions(("x-permissions" = json!(["support.tickets.reply"]))),
    operation_id = "remove_support_link",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket."), ("target" = String, Path, description = "The linked record, `<module>.<type>:<id>`.")),
    responses(
        (status = 200, description = "The links left.", body = Data<Vec<String>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply` (`permission_denied`), or it does not cover the ticket (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No ticket with this id in your scope; code `support.ticket_not_found`.", body = Problem),
    )
)]
pub(crate) async fn remove_link(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, target)): Path<(Uuid, String)>,
) -> Result<Data<Vec<String>>, Error> {
    principal.require(&TICKETS_REPLY)?;
    Ok(Data { data: service.remove_link(&tenant, &principal, id, &target).await? })
}

/// Where to hand the ticket over.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = EscalateSupportTicketRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct EscalateRequest {
    to: Escalation,
    /// The complaint in the customer's words (10 to 5000 characters); omitted: their first message.
    note: Option<String>,
}

/// Hand a ticket over as a consumer complaint or to the marketplace.
///
/// Requires `support.tickets.manage` covering the ticket. `consumer_complaint`: module `compliance`
/// opens a complaint case with legal deadlines; the ticket keeps `case_id` and closes as
/// `escalated`. Calling again (or with the same `Idempotency-Key`) never opens a second case.
/// `platform`: the marketplace operator's staff take it over; the ticket stays open.
#[utoipa::path(
    post,
    path = "/tickets/{id}/escalate",
    extensions(("x-permissions" = json!(["support.tickets.manage"]))),
    operation_id = "escalate_support_ticket",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = EscalateRequest,
    responses(
        (status = 200, description = "The ticket after the hand-over.", body = Data<TicketResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.manage` (`permission_denied`), or it does not cover the ticket (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No ticket with this id in your scope; code `support.ticket_not_found`.", body = Problem),
        (status = 409, description = "Already handed over to the platform (`support.already_escalated`), or the ticket is closed (`support.ticket_closed`).", body = Problem),
        (status = 422, description = "The complaint text is too short or long; see `errors`.", body = Problem),
        (status = 503, description = "Module `compliance` is not installed; code `support.complaints_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn escalate(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<EscalateRequest>,
) -> Result<Data<TicketResponse>, Error> {
    principal.require(&TICKETS_MANAGE)?;
    let ticket = service.escalate(&tenant, &principal, id, input.to, input.note).await?;
    Ok(Data { data: TicketResponse::new(ticket, true) })
}

/// A rating of the support.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = RateSupportTicketRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct RateRequest {
    /// Guests: the ticket token. Signed-in customers omit it.
    token: Option<String>,
    /// 1 (bad) to 5 (very good).
    rating: i16,
    /// Up to 1000 characters.
    note: Option<String>,
}

/// Rate the support of your ticket.
///
/// Any signed-in account, for its own ticket. Only once the ticket is solved or closed; rating again
/// replaces the rating.
#[utoipa::path(
    post,
    path = "/me/tickets/{id}/satisfaction",
    extensions(("x-permissions" = json!([]))),
    operation_id = "rate_my_support_ticket",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = RateRequest,
    responses(
        (status = 200, description = "The ticket with the rating.", body = Data<TicketResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no ticket with this id; code `support.ticket_not_found`.", body = Problem),
        (status = 409, description = "The ticket is not solved yet; code `support.not_solved`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn rate_mine(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RateRequest>,
) -> Result<Data<TicketResponse>, Error> {
    let viewer = Viewer::Customer(principal.user_id());
    let ticket = service.rate(&tenant, viewer, id, input.rating, input.note).await?;
    Ok(Data { data: TicketResponse::new(ticket, false) })
}

/// Rate the support of a guest ticket.
///
/// Public, with the ticket token. Only once the ticket is solved or closed.
#[utoipa::path(
    post,
    path = "/public/tickets/{id}/satisfaction",
    extensions(("x-public" = json!(true))),
    operation_id = "rate_public_support_ticket",
    security(()),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = RateRequest,
    responses(
        (status = 200, description = "The ticket with the rating.", body = Data<TicketResponse>),
        (status = 404, description = "No ticket with this id and token; code `support.ticket_not_found`.", body = Problem),
        (status = 409, description = "The ticket is not solved yet; code `support.not_solved`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn rate_public(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RateRequest>,
) -> Result<Data<TicketResponse>, Error> {
    service.by_token(&tenant, id, input.token.as_deref().unwrap_or_default()).await?;
    let ticket = service.rate(&tenant, Viewer::Guest, id, input.rating, input.note).await?;
    Ok(Data { data: TicketResponse::new(ticket, false) })
}

/// A saved reply staff insert into answers.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportMacro)]
pub(crate) struct MacroResponse {
    id: Uuid,
    /// The unit it belongs to; units below use it too.
    org_id: Uuid,
    name: String,
    /// Text per language (`vi`, `en`…), with `{customer_name}`, `{ticket_number}`, `{subject}`.
    #[schema(value_type = Object)]
    bodies: BTreeMap<String, String>,
    /// Approved for automatic answers.
    auto_reply: bool,
    /// Ways customers ask the question, per language (automatic answers).
    #[schema(value_type = Object)]
    questions: BTreeMap<String, Vec<String>>,
    approved_by: Option<Uuid>,
    approved_at: Option<Timestamp>,
    /// Automatic use stops after this day unless reviewed again (`YYYY-MM-DD`).
    review_due_on: Option<String>,
    updated_at: Timestamp,
}

impl From<SavedReply> for MacroResponse {
    fn from(row: SavedReply) -> Self {
        MacroResponse {
            id: row.id,
            org_id: row.org_id,
            name: row.name,
            bodies: serde_json::from_str(&row.bodies).unwrap_or_default(),
            auto_reply: row.auto_reply,
            questions: serde_json::from_str(&row.questions).unwrap_or_default(),
            approved_by: row.approved_by,
            approved_at: row.approved_at,
            review_due_on: row.review_due_on,
            updated_at: row.updated_at,
        }
    }
}

/// The unit whose saved replies to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct MacroQuery {
    org_id: Uuid,
}

/// List the saved replies usable at a unit.
///
/// Requires `support.tickets.read` at the unit; includes the saved replies of the units above it
/// (at most 500 in all). By name unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/macros",
    extensions(("x-permissions" = json!(["support.tickets.read"]))),
    operation_id = "list_support_macros",
    security(("bearer" = [])),
    params(MacroQuery, SortedPageQuery),
    responses(
        (status = 200, description = "One page of the saved replies, by name.", body = Page<MacroResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.read` (`permission_denied`), or it does not cover the unit (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "`org_id` is missing or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_macros(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<MacroQuery>,
    Query(page): Query<SortedPageQuery>,
) -> Result<Page<MacroResponse>, Error> {
    principal.require(&TICKETS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.saved_replies(&tenant, &principal, query.org_id).await?;
    // Tối đa 500 câu (đã kẹp trong truy vấn): phân trang trong bộ nhớ theo tên rồi ID, như mọi danh sách.
    let page = page_of(rows, |row| format!("{}\u{0}{}", row.name, row.id), &page)?;
    Ok(page.map(MacroResponse::from))
}

/// A saved reply to write.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SaveSupportMacroRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct MacroRequest {
    org_id: Uuid,
    /// 1 to 100 characters.
    name: String,
    /// Text per language; variables `{customer_name}`, `{ticket_number}`, `{subject}`.
    #[schema(value_type = Object, example = json!({"vi": "Chào {customer_name}, …", "en": "Hello {customer_name}, …"}))]
    bodies: BTreeMap<String, String>,
}

impl From<MacroRequest> for SavedReplyInput {
    fn from(input: MacroRequest) -> Self {
        SavedReplyInput { org_id: input.org_id, name: input.name, bodies: input.bodies }
    }
}

/// Create a saved reply.
///
/// Requires `support.settings.manage` at the unit.
#[utoipa::path(
    post,
    path = "/macros",
    extensions(("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "create_support_macro",
    security(("bearer" = [])),
    request_body = MacroRequest,
    responses(
        (status = 201, description = "Created.", body = Data<MacroResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_macro(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<MacroRequest>,
) -> Result<Created<MacroResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let row = service.save_reply(&tenant, &principal, None, input.into()).await?;
    Ok(Created::new(format!("/support/macros/{}", row.id), row.into()))
}

/// Replace a saved reply.
///
/// Requires `support.settings.manage` at its unit. Changing the text withdraws its approval for
/// automatic answers.
#[utoipa::path(
    put,
    path = "/macros/{id}",
    extensions(("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "update_support_macro",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the saved reply.")),
    request_body = MacroRequest,
    responses(
        (status = 200, description = "The saved reply.", body = Data<MacroResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), or it does not cover the unit (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No saved reply with this id; code `support.saved_reply_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_macro(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MacroRequest>,
) -> Result<Data<MacroResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let row = service.save_reply(&tenant, &principal, Some(id), input.into()).await?;
    Ok(Data { data: row.into() })
}

/// Delete a saved reply.
///
/// Requires `support.settings.manage` at its unit.
#[utoipa::path(
    delete,
    path = "/macros/{id}",
    extensions(("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "delete_support_macro",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the saved reply.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), or it does not cover the unit (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No saved reply with this id; code `support.saved_reply_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_macro(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    service.delete_reply(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A saved reply filled in for one ticket.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = RenderedSupportMacro)]
pub(crate) struct RenderedMacro {
    /// Text in the customer's language with the variables filled in; edit it, then send it as a
    /// message.
    body: String,
}

/// Fill a saved reply in for a ticket.
///
/// Requires `support.tickets.read` covering the ticket. Nothing is sent.
#[utoipa::path(
    post,
    path = "/tickets/{id}/macros/{macro_id}/render",
    extensions(("x-permissions" = json!(["support.tickets.read"]))),
    operation_id = "render_support_macro",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket."), ("macro_id" = Uuid, Path, description = "Id of the saved reply.")),
    responses(
        (status = 200, description = "The filled-in text.", body = Data<RenderedMacro>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such ticket in your scope (`support.ticket_not_found`) or saved reply usable for it (`support.saved_reply_not_found`).", body = Problem),
    )
)]
pub(crate) async fn render_macro(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, macro_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<RenderedMacro>, Error> {
    principal.require(&TICKETS_READ)?;
    let body = service.render_reply(&tenant, &principal, id, macro_id).await?;
    Ok(Data { data: RenderedMacro { body } })
}
