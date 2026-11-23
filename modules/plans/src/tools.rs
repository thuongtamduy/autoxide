//! Tool MCP của module `plans`: cùng service, cùng permission với REST. Không có tool cho thanh
//! toán hay đăng ký công khai (chỉ danh sách đăng ký cho nhân sự nền tảng, chỉ đọc).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::mcp::{NoInput, PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::{
    EntitlementsResponse, LimitUse, OverrideResponse, PlanRequest, PlanResponse,
    SubscriptionResponse, describe_entitlements, limit_uses, operator,
};
use crate::permissions::{
    PLANS_MANAGE, PLANS_READ, PROVISION, SUBSCRIPTIONS_MANAGE, SUBSCRIPTIONS_READ,
};
use crate::provisioning::ProvisioningResponse;
use crate::service::{PlansService, subscription_not_found};
use crate::signup_http::SignupResponse;

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListPlansInput {
    /// Maximum number of plans to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListSignupsInput {
    /// Only sign-ups in this state: `pending_email`, `awaiting_kyc`, `provisioning`, `completed`,
    /// `rejected` or `expired`.
    status: Option<String>,
    /// Maximum number of sign-ups to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanId {
    /// Id of the plan.
    id: Uuid,
}

// Trường phẳng (client MCP không đọc `$ref`), nên chép lại các trường của `PlanRequest`.
/// The whole new content of a plan, with its id.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdatePlanInput {
    /// Id of the plan.
    id: Uuid,
    /// Unique code, `[a-z][a-z0-9_-]{0,63}`.
    code: String,
    /// Name shown to customers, 1 to 200 characters.
    name: String,
    #[serde(default)]
    description: String,
    /// `base` or `addon`.
    kind: String,
    /// `draft` or `active`.
    status: String,
    #[serde(default)]
    is_default: bool,
    #[serde(default)]
    trial_days: i32,
    grace_days: i32,
    suspend_days: i32,
    retention_days: i32,
    renewal_lead_days: i32,
    /// `shared` or `dedicated`.
    placement: String,
    #[serde(default)]
    requires_kyc: bool,
    #[serde(default)]
    sample_data: bool,
    /// New tenants may choose this plan when they sign up by themselves.
    #[serde(default)]
    self_signup: bool,
    /// Modules to turn on.
    #[serde(default)]
    modules: Vec<String>,
    /// Features to turn on.
    #[serde(default)]
    features: Vec<String>,
    /// Limits by key; `null` is unlimited, a limit left out is 0.
    #[serde(default)]
    limits: std::collections::BTreeMap<String, Option<i64>>,
}

impl From<UpdatePlanInput> for crate::service::PlanInput {
    fn from(input: UpdatePlanInput) -> Self {
        crate::service::PlanInput {
            code: input.code,
            name: input.name,
            description: input.description,
            kind: input.kind,
            status: input.status,
            is_default: input.is_default,
            trial_days: input.trial_days,
            grace_days: input.grace_days,
            suspend_days: input.suspend_days,
            retention_days: input.retention_days,
            renewal_lead_days: input.renewal_lead_days,
            placement: input.placement,
            requires_kyc: input.requires_kyc,
            sample_data: input.sample_data,
            self_signup: input.self_signup,
            modules: input.modules,
            features: input.features,
            limits: input.limits,
        }
    }
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TenantInput {
    /// Id of the tenant.
    tenant_id: Uuid,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssignInput {
    /// Id of the tenant.
    tenant_id: Uuid,
    /// An active base plan.
    plan_id: Uuid,
    /// An active recurring price of that plan.
    price_id: Option<Uuid>,
    /// Start with the plan's trial when the tenant never had one; default `true`.
    trial: Option<bool>,
    /// Replace the add-ons with these active add-on plans, one of each; leave out to keep them.
    /// Use `PUT /api/v1/plans/tenants/{tenant_id}/subscription` for quantities above one.
    addon_plan_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct OverrideInput {
    /// Id of the tenant.
    tenant_id: Uuid,
    /// `module`, `feature` or `limit`.
    kind: String,
    /// Module name, feature key or limit key.
    key: String,
    /// Required with `module` / `feature`.
    enabled: Option<bool>,
    /// With `limit`: the new maximum; leave out for unlimited.
    max_value: Option<i64>,
    /// Why, 1 to 500 characters.
    reason: String,
    /// When the override stops applying (RFC 3339).
    expires_at: Option<Timestamp>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeInput {
    /// An active base plan.
    plan_id: Uuid,
    /// An active recurring price of that plan.
    price_id: Option<Uuid>,
    /// Where billing reminders go.
    billing_email: Option<String>,
    /// Your reference for this change, 1 to 200 characters.
    reference: String,
}

pub(crate) fn all(service: &Arc<PlansService>) -> Vec<Tool> {
    let mut tools = plan_tools(service);
    tools.extend(tenant_tools(service));
    let signups = service.clone();
    tools.push(
        Tool::new(
            "list_signups",
            "Platform staff: list tenant sign-ups (people who signed up a new tenant by themselves), \
             newest first, optionally by `status`; follow `pagination.next_cursor`.",
            move |_: TenantContext, principal: Principal, input: ListSignupsInput| {
                let service = signups.clone();
                async move {
                    operator(&principal, &SUBSCRIPTIONS_READ)?;
                    let page = PageInput::new(input.limit, input.cursor, input.order).page()?;
                    let status = crate::signup_http::check_status(input.status)?;
                    let page = service.list_signups(status, &page).await?;
                    Ok::<Page<SignupResponse>, Error>(
                        page.map(|row| SignupResponse::of(row, None)),
                    )
                }
            },
        )
        .requires(SUBSCRIPTIONS_READ)
        .read_only(),
    );
    let change = service.clone();
    tools.push(
        Tool::new(
            "change_subscription",
            "Move your tenant to another plan: a plan costing at least as much per month applies \
             at once (the rest of the period is billed), a cheaper one when the period ends.",
            move |tenant: TenantContext, principal: Principal, input: ChangeInput| {
                let service = change.clone();
                async move {
                    if input.reference.trim().is_empty() || input.reference.len() > 200 {
                        return Err(Error::validation(vec![
                            autoxide_kernel::error::FieldError::new(
                                "reference",
                                "length",
                                "must be 1 to 200 characters long",
                            ),
                        ]));
                    }
                    let change = crate::lifecycle::ChangeInput {
                        plan_id: input.plan_id,
                        price_id: input.price_id,
                        addons: None,
                        billing_email: input.billing_email,
                    };
                    // Như REST (luật kiến trúc 8; rà soát bảo mật 2026-11): gói mua qua kho ứng
                    // dụng chỉ đổi ở kho, không đổi ở đây.
                    service.ensure_not_store_billed(tenant.id()).await?;
                    service.change(Actor::from(&principal), tenant.id(), change).await?;
                    let row = service
                        .subscription(tenant.id())
                        .await?
                        .ok_or_else(subscription_not_found)?;
                    Ok::<_, Error>(SubscriptionResponse::from(row))
                }
            },
        )
        .requires(crate::permissions::BILLING_MANAGE),
    );
    tools
}

fn plan_tools(service: &Arc<PlansService>) -> Vec<Tool> {
    let (list, get, create, update, retire) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_plans",
            "Platform staff: list every plan (drafts and retired ones too) with modules, features, \
             limits and prices, newest first; follow `pagination.next_cursor`.",
            move |_: TenantContext, principal: Principal, input: ListPlansInput| {
                let service = list.clone();
                async move {
                    operator(&principal, &PLANS_READ)?;
                    let page = PageInput::new(input.limit, input.cursor, input.order).page()?;
                    let page = service.list_plans(&page).await?;
                    Ok::<Page<PlanResponse>, Error>(page.map(PlanResponse::from))
                }
            },
        )
        .requires(PLANS_READ)
        .read_only(),
        Tool::new(
            "get_plan",
            "Platform staff: get one plan.",
            move |_: TenantContext, principal: Principal, input: PlanId| {
                let service = get.clone();
                async move {
                    operator(&principal, &PLANS_READ)?;
                    Ok::<_, Error>(PlanResponse::from(service.get_plan(input.id).await?))
                }
            },
        )
        .requires(PLANS_READ)
        .read_only(),
        Tool::new(
            "create_plan",
            "Platform staff: create a plan from the modules, features and limits the installed \
             modules declare (see `kernel.get_module`).",
            move |_: TenantContext, principal: Principal, input: PlanRequest| {
                let service = create.clone();
                async move {
                    operator(&principal, &PLANS_MANAGE)?;
                    let row = service.create_plan(Actor::from(&principal), input.into()).await?;
                    Ok::<_, Error>(PlanResponse::from(row))
                }
            },
        )
        .requires(PLANS_MANAGE),
        Tool::new(
            "update_plan",
            "Platform staff: replace the whole content of a plan; tenants on it get the change \
             within the cache time.",
            move |_: TenantContext, principal: Principal, input: UpdatePlanInput| {
                let service = update.clone();
                async move {
                    operator(&principal, &PLANS_MANAGE)?;
                    let actor = Actor::from(&principal);
                    let id = input.id;
                    let row = service.update_plan(actor, id, input.into(), None).await?;
                    Ok::<_, Error>(PlanResponse::from(row))
                }
            },
        )
        .requires(PLANS_MANAGE),
        Tool::new(
            "retire_plan",
            "Platform staff: stop selling a plan; tenants on it keep it.",
            move |_: TenantContext, principal: Principal, input: PlanId| {
                let service = retire.clone();
                async move {
                    operator(&principal, &PLANS_MANAGE)?;
                    let row = service.retire_plan(Actor::from(&principal), input.id).await?;
                    Ok::<_, Error>(PlanResponse::from(row))
                }
            },
        )
        .requires(PLANS_MANAGE),
    ]
}

fn tenant_tools(service: &Arc<PlansService>) -> Vec<Tool> {
    let (entitlements, get, assign, set, usage) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    let (provision, provisioning) = (service.clone(), service.clone());
    vec![
        Tool::new(
            "get_entitlements",
            "What your tenant may use now: modules, features, limits with their use, and whether \
             the data is read-only. Check it before offering an action the plan may not include.",
            move |tenant: TenantContext, _: Principal, _: NoInput| {
                let service = entitlements.clone();
                async move {
                    let found: Result<EntitlementsResponse, Error> =
                        describe_entitlements(&service, tenant.id()).await;
                    found
                }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "get_subscription",
            "Platform staff: get a tenant's subscription and add-ons.",
            move |_: TenantContext, principal: Principal, input: TenantInput| {
                let service = get.clone();
                async move {
                    operator(&principal, &SUBSCRIPTIONS_READ)?;
                    let row = service
                        .subscription(TenantId::from_uuid(input.tenant_id))
                        .await?
                        .ok_or_else(subscription_not_found)?;
                    Ok::<_, Error>(SubscriptionResponse::from(row))
                }
            },
        )
        .requires(SUBSCRIPTIONS_READ)
        .read_only(),
        Tool::new(
            "assign_subscription",
            "Platform staff: put a tenant on a plan (with the trial the first time), optionally \
             replacing its add-ons.",
            move |_: TenantContext, principal: Principal, input: AssignInput| {
                let service = assign.clone();
                async move {
                    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
                    let tenant = TenantId::from_uuid(input.tenant_id);
                    let assign = crate::service::AssignInput {
                        plan_id: input.plan_id,
                        price_id: input.price_id,
                        trial: input.trial.unwrap_or(true),
                        addons: input
                            .addon_plan_ids
                            .map(|ids| ids.into_iter().map(|id| (id, 1)).collect()),
                    };
                    let row = service.assign(Actor::from(&principal), tenant, assign).await?;
                    Ok::<_, Error>(SubscriptionResponse::from(row))
                }
            },
        )
        .requires(SUBSCRIPTIONS_MANAGE),
        Tool::new(
            "set_override",
            "Platform staff: for one tenant, turn a module or feature on or off, or replace a \
             limit, with a reason and an optional end.",
            move |_: TenantContext, principal: Principal, input: OverrideInput| {
                let service = set.clone();
                async move {
                    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
                    let tenant = TenantId::from_uuid(input.tenant_id);
                    let item = crate::service::OverrideInput {
                        kind: input.kind,
                        key: input.key,
                        enabled: input.enabled,
                        max_value: input.max_value,
                        reason: input.reason,
                        expires_at: input.expires_at,
                    };
                    let row = service.set_override(&principal, tenant, item).await?;
                    Ok::<_, Error>(OverrideResponse::from(row))
                }
            },
        )
        .requires(SUBSCRIPTIONS_MANAGE),
        Tool::new(
            "list_usage",
            "Platform staff: a tenant's use of every limit (metered limits: this month).",
            move |_: TenantContext, principal: Principal, input: TenantInput| {
                let service = usage.clone();
                async move {
                    operator(&principal, &SUBSCRIPTIONS_READ)?;
                    let found: Vec<LimitUse> =
                        limit_uses(&service, TenantId::from_uuid(input.tenant_id)).await?;
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(SUBSCRIPTIONS_READ)
        .read_only(),
        Tool::new(
            "provision_tenant",
            "Platform staff: create a tenant on a plan — the tenant, its subscription, sample data \
             when asked, and an email invitation of its administrator with the role `admin`. Runs \
             in the background; follow it with `get_provisioning`.",
            move |_: TenantContext, principal: Principal, input: ProvisionInput| {
                let service = provision.clone();
                async move {
                    operator(&principal, &PROVISION)?;
                    let mut request = crate::api::ProvisionRequest::new(
                        input.slug,
                        input.name,
                        input.plan_id,
                        input.admin_email,
                    );
                    request.price_id = input.price_id;
                    request.parent_id = input.parent_id;
                    request.placement = input.placement;
                    request.admin_name = input.admin_name;
                    request.sample_data = input.sample_data;
                    request.trial = input.trial.unwrap_or(true);
                    let row =
                        service.start_provisioning(Actor::from(&principal), request, true).await?;
                    Ok::<_, Error>(ProvisioningResponse::from(row))
                }
            },
        )
        .requires(PROVISION),
        Tool::new(
            "get_provisioning",
            "Platform staff: how far a tenant provisioning went — its steps, and the error when it \
             failed.",
            move |_: TenantContext, principal: Principal, input: ProvisioningId| {
                let service = provisioning.clone();
                async move {
                    operator(&principal, &SUBSCRIPTIONS_READ)?;
                    Ok::<_, Error>(ProvisioningResponse::from(
                        service.provisioning(input.id).await?,
                    ))
                }
            },
        )
        .requires(SUBSCRIPTIONS_READ)
        .read_only(),
    ]
}

// Trường phẳng (client MCP không đọc `$ref`), như `ProvisioningRequest` của REST.
/// A tenant to create on a plan.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProvisionInput {
    /// Short name of the new tenant: 1 to 63 lower-case letters, digits and `-`.
    slug: String,
    /// Name of the new tenant, 1 to 200 characters.
    name: String,
    /// An active base plan.
    plan_id: Uuid,
    /// An active recurring price of that plan.
    price_id: Option<Uuid>,
    /// Parent tenant, for chains.
    parent_id: Option<Uuid>,
    /// Name of the placement for its data; required for a `dedicated` plan.
    placement: Option<String>,
    /// Who administers the new tenant (invited by email with the role `admin`).
    admin_email: String,
    /// Their name.
    admin_name: Option<String>,
    /// Add sample data of the modules the plan turns on; default: the plan's setting.
    sample_data: Option<bool>,
    /// Start with the plan's free trial; default `true`.
    trial: Option<bool>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProvisioningId {
    /// Id of the provisioning.
    id: Uuid,
}
