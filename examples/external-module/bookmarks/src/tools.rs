//! MCP tools of the module, for AI agents. Each tool has an input type (checked like a REST
//! body), the permission of the matching REST endpoint, and a body that is ONE call into the
//! service — a tool can never do what REST with the same permission cannot. The core names them
//! `bookmarks.<tool>`.

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

use crate::http::{BookmarkResponse, CreateBookmarkRequest};
use crate::permissions::{BOOKMARKS_READ, BOOKMARKS_WRITE};
use crate::service::BookmarkService;

/// A bookmark to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BookmarkId {
    /// Id of the bookmark.
    id: Uuid,
}

pub(crate) fn all(service: &Arc<BookmarkService>) -> Vec<Tool> {
    // Each tool keeps its own `Arc` to the service (cloning an `Arc` only bumps a counter).
    let (list, create, delete) = (service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_bookmarks",
            "List your bookmarks, newest first; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: PageInput| {
                let service = list.clone();
                async move {
                    let page = service.list(&tenant, &principal, &input.page()?).await?;
                    Ok::<Page<BookmarkResponse>, Error>(page.map(BookmarkResponse::from))
                }
            },
        )
        .requires(BOOKMARKS_READ)
        .read_only(),
        Tool::new(
            "create_bookmark",
            "Keep a link as a bookmark. You keep each link once.",
            move |tenant: TenantContext, principal: Principal, input: CreateBookmarkRequest| {
                let service = create.clone();
                async move {
                    let bookmark = service.create(&tenant, &principal, input.into()).await?;
                    Ok::<_, Error>(BookmarkResponse::from(bookmark))
                }
            },
        )
        .requires(BOOKMARKS_WRITE),
        Tool::new(
            "delete_bookmark",
            "Delete one of your bookmarks. It cannot be restored.",
            move |tenant: TenantContext, principal: Principal, input: BookmarkId| {
                let service = delete.clone();
                async move {
                    service.delete(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(serde_json::json!({ "deleted": input.id }))
                }
            },
        )
        .requires(BOOKMARKS_WRITE)
        .destructive(),
    ]
}
