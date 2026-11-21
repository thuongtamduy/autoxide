//! Gợi ý trả lời bằng AI: chỉ là bản nháp (D-109).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::ai::Draft;
use crate::permissions::TICKETS_REPLY;
use crate::service::SupportService;

/// A draft answer staff review before sending. Never sent by itself.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportReplyDraft)]
pub(crate) struct DraftResponse {
    id: Uuid,
    ticket_id: Uuid,
    body: String,
    /// Written by AI: send it with `ai_assisted: true` so it is labelled.
    ai_assisted: bool,
    created_at: Timestamp,
}

impl From<Draft> for DraftResponse {
    fn from(row: Draft) -> Self {
        DraftResponse {
            id: row.id,
            ticket_id: row.ticket_id,
            body: row.body,
            ai_assisted: row.ai_assisted,
            created_at: row.created_at,
        }
    }
}

/// Draft an answer with AI.
///
/// Requires `support.tickets.reply` covering the ticket. The model reads the ticket's messages
/// (emails and phone numbers removed) and the saved replies of the unit; the result is only a
/// draft — edit it, then send it as a message with `ai_assisted: true`.
#[utoipa::path(
    post,
    path = "/tickets/{id}/reply-suggestions",
    extensions(("x-permissions" = json!(["support.tickets.reply"]))),
    operation_id = "suggest_support_reply",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    responses(
        (status = 200, description = "The draft.", body = Data<DraftResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply` (`permission_denied`), or it does not cover the ticket (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No ticket with this id in your scope; code `support.ticket_not_found`.", body = Problem),
        (status = 502, description = "The AI provider failed; codes `ai.provider_unavailable`, `ai.output_invalid`.", body = Problem),
        (status = 503, description = "AI is not set up; codes `support.ai_unavailable`, `ai.disabled`.", body = Problem),
    )
)]
pub(crate) async fn suggest_reply(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<DraftResponse>, Error> {
    principal.require(&TICKETS_REPLY)?;
    let draft = service.suggest_reply(&tenant, &principal, id).await?;
    Ok(Data { data: draft.into() })
}

/// List the drafts of a ticket.
///
/// Requires `support.tickets.reply` covering the ticket. Newest first.
#[utoipa::path(
    get,
    path = "/tickets/{id}/reply-drafts",
    extensions(("x-permissions" = json!(["support.tickets.reply"]))),
    operation_id = "list_support_reply_drafts",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    responses(
        (status = 200, description = "The drafts.", body = Data<Vec<DraftResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No ticket with this id in your scope; code `support.ticket_not_found`.", body = Problem),
    )
)]
pub(crate) async fn list_drafts(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<DraftResponse>>, Error> {
    principal.require(&TICKETS_REPLY)?;
    let drafts = service.drafts(&tenant, &principal, id).await?;
    Ok(Data { data: drafts.into_iter().map(DraftResponse::from).collect() })
}
