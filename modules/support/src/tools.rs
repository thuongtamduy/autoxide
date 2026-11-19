//! Tool MCP của module `support`. Mỗi tool: một kiểu đầu vào, permission như endpoint REST tương
//! ứng, thân là lời gọi service — không có logic riêng (luật kiến trúc 8).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::views::{MessageResponse, TicketDetail, TicketResponse};
use crate::messages::NewMessage;
use crate::permissions::{TICKETS_READ, TICKETS_REPLY};
use crate::service::{SupportService, Viewer};
use crate::status::{Priority, Resolution, Status};
use crate::tickets::{TicketChanges, TicketFilter};

/// Filters of the ticket list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ListSupportTicketsInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListInput {
    /// `open`, `pending_agent`, `pending_customer`, `solved` or `closed`.
    status: Option<String>,
    /// `true`: only tickets assigned to you.
    #[serde(default)]
    mine: bool,
    /// Only tickets linked to this record, `<module>.<type>:<id>`.
    link: Option<String>,
    /// Text in the number or subject.
    q: Option<String>,
    /// Maximum number of tickets, 1 to 200 (default 50).
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// A ticket to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SupportTicketIdInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct TicketId {
    /// Id of the ticket.
    id: Uuid,
}

/// A message to add.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SupportMessageInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageInput {
    /// Id of the ticket.
    id: Uuid,
    /// Plain text.
    body: String,
    /// Colleagues to notify (internal notes only).
    #[serde(default)]
    mentions: Vec<Uuid>,
}

/// Changes to a ticket; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = UpdateSupportTicketInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateInput {
    id: Uuid,
    /// `open`, `pending_agent`, `pending_customer`, `solved` or `closed`.
    status: Option<String>,
    /// With `solved` / `closed`: `answered`, `duplicate`, `spam` or `no_reply`.
    resolution: Option<String>,
    /// `low`, `normal`, `high` or `urgent`.
    priority: Option<String>,
    category: Option<String>,
    /// Assign to this user (yourself, or someone who answers tickets of the unit).
    assignee_user_id: Option<Uuid>,
}

/// A record to link.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = LinkSupportTicketInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkInput {
    id: Uuid,
    /// `<module>.<type>:<id>`.
    target: String,
}

pub(crate) fn all(service: &Arc<SupportService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let s = || service.clone();
    let (list, get, reply, note, update, link, suggest) = (s(), s(), s(), s(), s(), s(), s());
    vec![
        Tool::new(
            "list_tickets",
            "List the support tickets in your scope, newest first; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: ListInput| {
                let service = list.clone();
                async move {
                    let filter = TicketFilter {
                        status: parsed(input.status.as_deref(), "status", Status::parse)?,
                        assignee: input.mine.then_some(Some(principal.user_id())),
                        link: input.link,
                        q: input.q,
                        ..TicketFilter::default()
                    };
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let page = service.list(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<TicketResponse>, Error>(page.map(|ticket| TicketResponse::new(ticket, true)))
                }
            },
        )
        .requires(TICKETS_READ)
        .read_only(),
        Tool::new(
            "get_ticket",
            "Get a support ticket with its messages, internal notes and linked records.",
            move |tenant: TenantContext, principal: Principal, input: TicketId| {
                let service = get.clone();
                async move {
                    let viewer = Viewer::Staff(&principal);
                    let ticket = service.get(&tenant, viewer, input.id).await?;
                    let messages = service.messages(&tenant, viewer, &ticket).await?;
                    let links = service.links(&tenant, ticket.id).await?;
                    let mut detail = TicketDetail::new(ticket, messages, true);
                    detail.links = links;
                    Ok::<_, Error>(detail)
                }
            },
        )
        .requires(TICKETS_READ)
        .read_only(),
        Tool::new(
            "reply_ticket",
            "Answer the customer on a support ticket (they are notified; the ticket waits for them).",
            move |tenant: TenantContext, principal: Principal, input: MessageInput| {
                let service = reply.clone();
                async move {
                    let new = NewMessage { body: input.body, ..NewMessage::default() };
                    let (_, message) = service.add_message(&tenant, Viewer::Staff(&principal), input.id, new).await?;
                    Ok::<_, Error>(MessageResponse::from(message))
                }
            },
        )
        .requires(TICKETS_REPLY),
        Tool::new(
            "add_internal_note",
            "Add an internal note (staff only) to a support ticket; mentioned colleagues are notified.",
            move |tenant: TenantContext, principal: Principal, input: MessageInput| {
                let service = note.clone();
                async move {
                    let new = NewMessage {
                        body: input.body,
                        internal: true,
                        mentions: input.mentions,
                        ..NewMessage::default()
                    };
                    let (_, message) = service.add_message(&tenant, Viewer::Staff(&principal), input.id, new).await?;
                    Ok::<_, Error>(MessageResponse::from(message))
                }
            },
        )
        .requires(TICKETS_REPLY),
        Tool::new(
            "update_ticket",
            "Change status, priority, category or assignee of a support ticket.",
            move |tenant: TenantContext, principal: Principal, input: UpdateInput| {
                let service = update.clone();
                async move {
                    let changes = TicketChanges {
                        status: parsed(input.status.as_deref(), "status", Status::parse)?,
                        resolution: parsed(input.resolution.as_deref(), "resolution", Resolution::parse)?,
                        priority: parsed(input.priority.as_deref(), "priority", Priority::parse)?,
                        category: input.category,
                        assignee_user_id: input.assignee_user_id.map(Some),
                        team_id: None,
                    };
                    let ticket = service.update(&tenant, &principal, input.id, changes, None).await?;
                    Ok::<_, Error>(TicketResponse::new(ticket, true))
                }
            },
        )
        .requires(TICKETS_REPLY),
        Tool::new(
            "link_ticket",
            "Link a support ticket to a record of another module (an order, a product…).",
            move |tenant: TenantContext, principal: Principal, input: LinkInput| {
                let service = link.clone();
                async move {
                    let links = service.add_link(&tenant, &principal, input.id, &input.target).await?;
                    Ok::<_, Error>(serde_json::json!({"links": links}))
                }
            },
        )
        .requires(TICKETS_REPLY),
        Tool::new(
            "suggest_reply",
            "Draft an answer for a support ticket with AI. Nothing is sent: review the draft, then send it with `reply_ticket`.",
            move |tenant: TenantContext, principal: Principal, input: TicketId| {
                let service = suggest.clone();
                async move {
                    let draft = service.suggest_reply(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(crate::http::ai::DraftResponse::from(draft))
                }
            },
        )
        .requires(TICKETS_REPLY)
        .read_only(),
    ]
}

/// Giá trị chữ của tool thành kiểu của module; sai thì lỗi đầu vào như REST.
fn parsed<T>(
    raw: Option<&str>,
    field: &'static str,
    parse: fn(&str) -> Option<T>,
) -> Result<Option<T>, Error> {
    raw.map(|raw| {
        parse(raw).ok_or_else(|| {
            Error::validation(vec![autoxide_kernel::error::FieldError::new(
                field,
                "invalid",
                "is not one of the allowed values",
            )])
        })
    })
    .transpose()
}
