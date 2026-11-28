//! Tool MCP cho nhóm nút (bước 38, D-232): cùng service, cùng permission với REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::groups::GroupView;
use crate::http_groups::{AddMembersRequest, CreateGroupRequest, GroupResponse};
use crate::permissions::{GROUPS_MANAGE, UNITS_READ};
use crate::service::OrgService;

/// Which unit groups to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListGroupsInput {
    /// Words in the name; accents and case are ignored.
    q: Option<String>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// Units to add to a group.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddMembersInput {
    /// Id of the group.
    id: Uuid,
    /// 1 to 200 unit ids of this tenant; units already in the group are skipped.
    org_ids: Vec<Uuid>,
}

/// A unit to remove from a group.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoveMemberInput {
    /// Id of the group.
    id: Uuid,
    /// Id of the unit.
    org_id: Uuid,
}

/// Result of adding units to a group.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct AddedOutput {
    /// Units that were not in the group before.
    added: u64,
    /// Units in the group now.
    member_count: u64,
}

/// Result of removing a unit.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct RemovedOutput {
    /// Always `true`: the unit is not in the group (any more).
    removed: bool,
}

pub(crate) fn all(service: &Arc<OrgService>) -> Vec<Tool> {
    let (list, create, add, remove) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_groups",
            "List the unit groups of the tenant (free sets of units such as \"stores in the north\", \
             used to filter and group reports). `member_count` counts only units in your scope.",
            move |tenant: TenantContext, principal: Principal, input: ListGroupsInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, input.order)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let found =
                        service.list_groups(&tenant, &principal, input.q.as_deref(), &page).await?;
                    Ok::<Page<GroupResponse>, Error>(found.map(GroupResponse::from))
                }
            },
        )
        .requires(UNITS_READ)
        .read_only(),
        Tool::new(
            "create_group",
            "Create a unit group for reports. A group grants no access to its units.",
            move |tenant: TenantContext, principal: Principal, input: CreateGroupRequest| {
                let service = create.clone();
                async move {
                    let row = service
                        .create_group(&tenant, &principal, &input.name, input.description.as_deref())
                        .await?;
                    let view = GroupView { row, member_count: 0, excluded_org_count: 0 };
                    Ok::<_, Error>(GroupResponse::from(view))
                }
            },
        )
        .requires(GROUPS_MANAGE),
        Tool::new(
            "add_group_members",
            "Add units of the tenant (any branch) to a unit group; units already in it are skipped.",
            move |tenant: TenantContext, principal: Principal, input: AddMembersInput| {
                let service = add.clone();
                async move {
                    let request = AddMembersRequest { org_ids: input.org_ids };
                    let added =
                        service.add_members(&tenant, &principal, input.id, &request.org_ids).await?;
                    Ok::<_, Error>(AddedOutput {
                        added: added.added,
                        member_count: added.member_count,
                    })
                }
            },
        )
        .requires(GROUPS_MANAGE),
        Tool::new(
            "remove_group_member",
            "Remove one unit from a unit group (nothing happens when it is not in the group).",
            move |tenant: TenantContext, principal: Principal, input: RemoveMemberInput| {
                let service = remove.clone();
                async move {
                    service.remove_member(&tenant, &principal, input.id, input.org_id).await?;
                    Ok::<_, Error>(RemovedOutput { removed: true })
                }
            },
        )
        .requires(GROUPS_MANAGE)
        .requires_approval_for_agent(),
    ]
}
