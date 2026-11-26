//! MCP tools of the sandbox (D-232): an agent tries a configuration in a sandbox, then pushes it across.
//! There is no tool to delete a sandbox (REST only, with a human).

use std::sync::Arc;

use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::mailbox::SandboxMessage;
use super::service::{NewSandbox, Sandbox, SandboxCreated, TenantSandboxes};
use super::{SANDBOXES_MANAGE, SANDBOXES_READ};
use crate::access::Principal;
use crate::error::Error;
use crate::mcp::{PageInput, Tool};
use crate::pagination::Page;
use crate::tenancy::TenantContext;

/// Which sandboxes to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListInput {
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// Which kept messages of the current sandbox to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessagesInput {
    /// Only this channel: `partner`, `sms`, `zns`, `push`, `mail` or `webhook`.
    channel: Option<String>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// Records of a sandbox to promote.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromoteToolInput {
    /// Id of the sandbox.
    id: Uuid,
    /// 1 to 100 records of the sandbox, each `<kind>:<id>` with a kind from
    /// `kernel.list_promotable`, e.g. `content.page:0190…`.
    items: Vec<String>,
}

/// Which page of promotable kinds.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromotableInput {
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `asc` (by kind, the default) or `desc`.
    order: Option<String>,
    /// `true` to also get `pagination.total` (the number of items) on the first page.
    count: Option<bool>,
}

/// A new sandbox of your tenant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateInput {
    /// 1 to 200 characters.
    name: String,
    /// Products to copy (0 by default).
    products: Option<u32>,
    /// Role of your tenant whose holders may enter the sandbox; the `admin` role when omitted.
    role_id: Option<Uuid>,
}

/// A sandbox to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SandboxId {
    /// Id of the sandbox.
    id: Uuid,
}

pub(crate) fn all(service: Arc<TenantSandboxes>) -> Vec<Tool> {
    let (list, create, extend) = (service.clone(), service.clone(), service.clone());
    let (kinds, promote) = (service.clone(), service);
    vec![
        Tool::new(
            "list_sandboxes",
            "List the sandboxes (test copies) of your tenant, newest first.",
            move |tenant: TenantContext, principal: Principal, input: ListInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, input.order)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let source = tenant.id().as_uuid();
                    let found = service.list(&tenant, &principal, source, &page).await?;
                    Ok::<Page<Sandbox>, Error>(found)
                }
            },
        )
        .requires(SANDBOXES_READ)
        .read_only(),
        Tool::new(
            "create_sandbox",
            "Create a sandbox of your tenant: a test copy with its configuration and no personal \
             data, where partners are simulated and mail and webhooks do not leave. Enter it with \
             the header `Autoxide-Tenant: <id>` once it is active.",
            move |tenant: TenantContext, principal: Principal, input: CreateInput| {
                let service = create.clone();
                async move {
                    let new = NewSandbox {
                        name: input.name,
                        products: input.products,
                        role_id: input.role_id,
                    };
                    let source = tenant.id().as_uuid();
                    let created = service.create(&tenant, &principal, source, new).await?;
                    Ok::<SandboxCreated, Error>(created)
                }
            },
        )
        .requires(SANDBOXES_MANAGE),
        Tool::new(
            "extend_sandbox",
            "Extend a sandbox of your tenant: it is then deleted `AUTOXIDE_TENANCY_SANDBOX_DAYS` \
             days from now.",
            move |tenant: TenantContext, principal: Principal, input: SandboxId| {
                let service = extend.clone();
                async move { service.extend(&tenant, &principal, input.id).await }
            },
        )
        .requires(SANDBOXES_MANAGE),
        Tool::new(
            "list_sandbox_messages",
            "Inside a sandbox: list what it kept instead of sending out — simulated partner calls \
             (payments, e-invoices, identity checks), SMS, Zalo and push messages, mail to people \
             who cannot enter the sandbox and webhooks to URLs not allowed. Newest first.",
            move |tenant: TenantContext, principal: Principal, input: MessagesInput| async move {
                let page = PageInput::new(input.limit, input.cursor, input.order)
                    .with_count(input.count.unwrap_or(false))
                    .page()?;
                let found =
                    super::mailbox::list(&tenant, &principal, input.channel.as_deref(), &page)
                        .await?;
                Ok::<Page<SandboxMessage>, Error>(found)
            },
        )
        .requires(SANDBOXES_READ)
        .read_only(),
        Tool::new(
            "list_promotable",
            "List the kinds of records that can be promoted from a sandbox to your tenant.",
            move |_: TenantContext, principal: Principal, input: PromotableInput| {
                let service = kinds.clone();
                async move {
                    // Cùng kiểu tham số trang và cùng hàm với REST.
                    let page = PageInput::new(input.limit, input.cursor, input.order)
                        .with_count(input.count.unwrap_or(false))
                        .natural_page()?;
                    super::http::promotable_page(&service, &principal, &page)
                }
            },
        )
        .requires(SANDBOXES_READ)
        .read_only(),
        Tool::new(
            "promote_sandbox",
            "Promote records tried in a sandbox to your tenant as drafts (nothing is published or \
             enabled; what is live is never overwritten). Call it in the real tenant. Each record \
             gets `created_draft`, `updated_draft`, `unchanged` or `conflict`.",
            move |tenant: TenantContext, principal: Principal, input: PromoteToolInput| {
                let service = promote.clone();
                async move {
                    let items = super::promote::parse_items(&input.items)?;
                    let request = super::promote::PromoteInput { items };
                    service.promote(&tenant, &principal, input.id, request).await
                }
            },
        )
        .requires(SANDBOXES_MANAGE),
    ]
}
