//! Tool MCP của module `organizations`. Mỗi tool: permission như endpoint REST tương ứng, thân là
//! MỘT lời gọi service (luật kiến trúc 8).

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

use crate::http::{CreateUnitRequest, UnitResponse};
use crate::permissions::{UNITS_MANAGE, UNITS_READ, UNITS_WRITE};
use crate::service::{OrgService, UnitChanges, UnitFilter};

/// Which units to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListUnitsInput {
    /// Only the units directly below this unit.
    parent_id: Option<Uuid>,
    /// Only units of this type (code).
    unit_type: Option<String>,
    /// `pending`, `active`, `suspended` or `archived`.
    status: Option<String>,
    /// Words in the name; accents and case are ignored.
    q: Option<String>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
}

/// A unit to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnitId {
    /// Id of the unit.
    id: Uuid,
}

/// Changes to a unit; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateUnitInput {
    /// Id of the unit.
    id: Uuid,
    /// New name, 1 to 200 characters.
    name: Option<String>,
    /// New internal code, 1 to 64 characters; an empty string removes it.
    code: Option<String>,
}

/// Where to move a unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveUnitInput {
    /// Id of the unit to move, with everything below it.
    id: Uuid,
    /// The new parent.
    parent_id: Uuid,
}

/// A new status for a unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnitStatusInput {
    /// Id of the unit.
    id: Uuid,
    /// `active`, `suspended` or `archived`.
    status: String,
    /// Why, up to 500 characters.
    reason: Option<String>,
}

pub(crate) fn all(service: &Arc<OrgService>) -> Vec<Tool> {
    let (list, get, create, update, relocate, status) = (
        service.clone(),
        service.clone(),
        service.clone(),
        service.clone(),
        service.clone(),
        service.clone(),
    );
    vec![
        Tool::new(
            "list_units",
            "List the organization units in your scope (head office, regions, stores…), newest \
             first; filter by parent, type, status or name; follow `pagination.next_cursor`.",
            move |tenant: TenantContext, principal: Principal, input: ListUnitsInput| {
                let service = list.clone();
                async move {
                    let filter = UnitFilter {
                        parent_id: input.parent_id,
                        unit_type: input.unit_type,
                        status: input.status,
                        q: input.q,
                    };
                    let page = PageInput::new(input.limit, input.cursor, input.order).page()?;
                    let page = service.list(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<UnitResponse>, Error>(page.map(UnitResponse::from))
                }
            },
        )
        .requires(UNITS_READ)
        .read_only(),
        Tool::new(
            "get_unit",
            "Get one organization unit in your scope.",
            move |tenant: TenantContext, principal: Principal, input: UnitId| {
                let service = get.clone();
                async move {
                    let row = service.get(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(UnitResponse::from(row))
                }
            },
        )
        .requires(UNITS_READ)
        .read_only(),
        Tool::new(
            "create_unit",
            "Create an organization unit under a parent you can write to. It is active when you \
             can also manage units there, otherwise pending until approved.",
            move |tenant: TenantContext, principal: Principal, input: CreateUnitRequest| {
                let service = create.clone();
                async move {
                    let row = service.create(&tenant, &principal, input.into()).await?;
                    Ok::<_, Error>(UnitResponse::from(row))
                }
            },
        )
        .requires(UNITS_WRITE),
        Tool::new(
            "update_unit",
            "Rename an organization unit or change its internal code.",
            move |tenant: TenantContext, principal: Principal, input: UpdateUnitInput| {
                let service = update.clone();
                async move {
                    let changes = UnitChanges {
                        name: input.name,
                        code: input
                            .code
                            .map(|code| Some(code).filter(|code| !code.trim().is_empty())),
                    };
                    let row = service.update(&tenant, &principal, input.id, changes, None).await?;
                    Ok::<_, Error>(UnitResponse::from(row))
                }
            },
        )
        .requires(UNITS_WRITE),
        Tool::new(
            "move_unit",
            "Move an organization unit, with everything below it, under another parent.",
            move |tenant: TenantContext, principal: Principal, input: MoveUnitInput| {
                let service = relocate.clone();
                async move {
                    let row =
                        service.relocate(&tenant, &principal, input.id, input.parent_id).await?;
                    Ok::<_, Error>(UnitResponse::from(row))
                }
            },
        )
        .requires(UNITS_WRITE),
        Tool::new(
            "change_unit_status",
            "Approve, suspend, archive or reactivate an organization unit. While a unit is not \
             active, nothing can be written to it or to the units below it.",
            move |tenant: TenantContext, principal: Principal, input: UnitStatusInput| {
                let service = status.clone();
                async move {
                    let row = service
                        .change_status(&tenant, &principal, input.id, &input.status, input.reason)
                        .await?;
                    Ok::<_, Error>(UnitResponse::from(row))
                }
            },
        )
        .requires(UNITS_MANAGE)
        .destructive(),
    ]
}
