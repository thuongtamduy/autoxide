//! Tool MCP của module `notes`. Mỗi tool: một kiểu đầu vào (kiểm như thân JSON của
//! REST), permission như endpoint REST tương ứng, thân là MỘT lời gọi service — không
//! có logic riêng (luật kiến trúc 8).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{Schema, Validate};
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use uuid::Uuid;

use crate::http::{CreateNoteRequest, NoteResponse};
use crate::permissions::{NOTES_READ, NOTES_WRITE};
use crate::service::{NoteChanges, NoteService};

/// A note to act on.
#[derive(Debug, Deserialize, Validate, Schema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoteId {
    /// Id of the note.
    id: Uuid,
}

/// Changes to a note; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, Schema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateNoteInput {
    /// Id of the note.
    id: Uuid,
    /// New title, 1 to 200 characters.
    title: Option<String>,
    /// New text, up to 10000 characters.
    body: Option<String>,
}

pub(crate) fn all(service: &Arc<NoteService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (list, get, create, update, delete) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_notes",
            "List the notes in your scope, newest first; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: PageInput| {
                let service = list.clone();
                async move {
                    let page = service.list(&tenant, &principal, &input.page()?).await?;
                    Ok::<Page<NoteResponse>, Error>(page.map(NoteResponse::from))
                }
            },
        )
        .requires(NOTES_READ)
        .read_only(),
        Tool::new(
            "get_note",
            "Get a note in your scope.",
            move |tenant: TenantContext, principal: Principal, input: NoteId| {
                let service = get.clone();
                async move {
                    let note = service.get(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(NoteResponse::from(note))
                }
            },
        )
        .requires(NOTES_READ)
        .read_only(),
        Tool::new(
            "create_note",
            "Create a note. Titles are unique among your notes.",
            move |tenant: TenantContext, principal: Principal, input: CreateNoteRequest| {
                let service = create.clone();
                async move {
                    let note = service.create(&tenant, &principal, input.into()).await?;
                    Ok::<_, Error>(NoteResponse::from(note))
                }
            },
        )
        .requires(NOTES_WRITE),
        Tool::new(
            "update_note",
            "Change the title or text of a note in your scope.",
            move |tenant: TenantContext, principal: Principal, input: UpdateNoteInput| {
                let service = update.clone();
                async move {
                    let changes = NoteChanges { title: input.title, body: input.body };
                    let note = service.update(&tenant, &principal, input.id, changes, None).await?;
                    Ok::<_, Error>(NoteResponse::from(note))
                }
            },
        )
        .requires(NOTES_WRITE),
        Tool::new(
            "delete_note",
            "Delete a note in your scope. It cannot be restored.",
            move |tenant: TenantContext, principal: Principal, input: NoteId| {
                let service = delete.clone();
                async move {
                    service.delete(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(serde_json::json!({ "deleted": input.id }))
                }
            },
        )
        .requires(NOTES_WRITE)
        .destructive(),
    ]
}
