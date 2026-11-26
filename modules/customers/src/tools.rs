//! Tool MCP của module `customers`. Mỗi tool: một kiểu đầu vào (kiểm như thân JSON của REST),
//! permission như endpoint REST tương ứng, thân là MỘT lời gọi service — không có logic riêng (luật
//! kiến trúc 8). Hồ sơ trả về luôn mang liên hệ đã che; chỉ `reveal_contact` trả đủ và ghi audit
//! (D-226). Gắn nhãn và ghi chú lên hồ sơ dùng tool chung của kernel (loại `customers.customer`).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::views::{
    CustomerContact, CustomerGroupPreview, CustomerGroupResponse, CustomerResponse, try_page,
};
use crate::permissions::{CONTACT_REVEAL, READ, SEGMENTS_MANAGE};
use crate::profiles::ProfileFilter;
use crate::segments::{GroupInput, GroupRule};
use crate::service::CustomersService;

/// Which customers to find.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchCustomersInput {
    /// A name (accents do not matter), a phone number or an email.
    q: Option<String>,
    /// Only members of this customer group.
    group_id: Option<Uuid>,
    /// Only this membership tier code.
    tier: Option<String>,
    /// 1 to 200; default 50.
    limit: Option<u32>,
    /// `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// One customer.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CustomerIdInput {
    /// Id of the customer profile.
    id: Uuid,
}

/// Why the full contact details are needed.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RevealContactInput {
    /// Id of the customer profile.
    id: Uuid,
    /// Reason kept in the audit log, up to 200 characters.
    reason: Option<String>,
}

/// A page of groups.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListGroupsInput {
    /// 1 to 200; default 50.
    limit: Option<u32>,
    /// `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// A group to create (no `id`) or change (with `id`).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpsertGroupInput {
    /// Id of the group to change; omitted: create one.
    id: Option<Uuid>,
    /// 1 to 100 characters, unique in the shop (required to create).
    name: Option<String>,
    description: Option<String>,
    /// `manual` or `rule` (only when creating; default `manual`).
    kind: Option<String>,
    /// Rule of a rule group; all given conditions must hold: `total_spent_min` (smallest currency
    /// unit), `orders_count_min`, `last_order_within_days` (1 to 3650), `tier_in` (tier codes),
    /// `tags_any` (tag ids), `org_in` (unit ids).
    #[schema(value_type = Option<Object>)]
    rule: Option<GroupRule>,
    /// Customer group code for price lists, e.g. `vip`.
    price_group: Option<String>,
    /// Unit of a new group; omitted: the root unit.
    org_id: Option<Uuid>,
}

/// One group.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupIdInput {
    /// Id of the group.
    id: Uuid,
}

pub(crate) fn all(service: &Arc<CustomersService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (search, get, reveal, groups, upsert, preview) = (
        service.clone(),
        service.clone(),
        service.clone(),
        service.clone(),
        service.clone(),
        service.clone(),
    );
    vec![
        Tool::new(
            "search_customers",
            "Find customers in your scope by name, phone number or email, or list a group's or tier's \
             customers. Phone and email are masked.",
            move |tenant: TenantContext, principal: Principal, input: SearchCustomersInput| {
                let service = search.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let filter = ProfileFilter {
                        q: input.q,
                        group_id: input.group_id,
                        tier: input.tier,
                        ..ProfileFilter::default()
                    };
                    try_page(
                        service.list(&tenant, &principal, &filter, &page).await?,
                        CustomerResponse::new,
                    )
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "get_customer",
            "Get one customer profile: totals, tier, custom fields. Phone and email are masked.",
            move |tenant: TenantContext, principal: Principal, input: CustomerIdInput| {
                let service = get.clone();
                async move { CustomerResponse::new(service.get(&tenant, &principal, input.id).await?) }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "reveal_contact",
            "Show the full phone number and email of a customer. Every call is written to the audit log; \
             give a reason.",
            move |tenant: TenantContext, principal: Principal, input: RevealContactInput| {
                let service = reveal.clone();
                async move {
                    let contact =
                        service.reveal_contact(&tenant, &principal, input.id, input.reason).await?;
                    Ok(CustomerContact { phone: contact.phone, email: contact.email })
                }
            },
        )
        .requires(CONTACT_REVEAL),
        Tool::new(
            "list_groups",
            "List customer groups (manual or rule-based) in your scope.",
            move |tenant: TenantContext, principal: Principal, input: ListGroupsInput| {
                let service = groups.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    Ok(service
                        .list_groups(&tenant, &principal, &page)
                        .await?
                        .map(CustomerGroupResponse::from))
                }
            },
        )
        .requires(READ)
        .or_requires(SEGMENTS_MANAGE)
        .read_only(),
        Tool::new(
            "upsert_group",
            "Create a customer group (no `id`) or change one. A rule group's members follow its rule \
             shortly after; check with `preview_group` first.",
            move |tenant: TenantContext, principal: Principal, input: UpsertGroupInput| {
                let service = upsert.clone();
                async move {
                    let fields = GroupInput {
                        name: input.name,
                        description: input.description,
                        kind: input.kind,
                        rule: input.rule,
                        price_group: input.price_group,
                        org_id: input.org_id,
                    };
                    let row = match input.id {
                        Some(id) => service.update_group(&tenant, &principal, id, fields).await?,
                        None => service.create_group(&tenant, &principal, fields).await?,
                    };
                    Ok(CustomerGroupResponse::from(row))
                }
            },
        )
        .requires(SEGMENTS_MANAGE),
        Tool::new(
            "preview_group",
            "Count the customers a group matches now (rule groups: matching the rule; manual groups: \
             members).",
            move |tenant: TenantContext, principal: Principal, input: GroupIdInput| {
                let service = preview.clone();
                async move {
                    let matching = service.preview_group(&tenant, &principal, input.id).await?;
                    Ok(CustomerGroupPreview { matching })
                }
            },
        )
        .requires(READ)
        .or_requires(SEGMENTS_MANAGE)
        .read_only(),
    ]
}
