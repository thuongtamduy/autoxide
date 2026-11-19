//! Tool MCP của sandbox (D-232): agent thử cấu hình trong sandbox rồi đẩy sang. Không có tool xoá
//! sandbox (chỉ REST, có người).

use std::sync::Arc;

use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

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
    let (list, create, extend) = (service.clone(), service.clone(), service);
    vec![
        Tool::new(
            "list_sandboxes",
            "List the sandboxes (test copies) of your tenant, newest first.",
            move |tenant: TenantContext, principal: Principal, input: ListInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, input.order).page()?;
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
    ]
}
