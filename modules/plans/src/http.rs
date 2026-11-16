//! Route REST của module `plans`, dưới `/api/v1/plans`.
//!
//! "Vận hành" (gói, thuê bao của mọi khách) cần permission giữ ở khách nền tảng; "khách"
//! (`billing`) là permission ở khách hiện tại.

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::entitlement::TenantAccess;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{TenantContext, TenantId, require_platform};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::entity::{grant, price};
use crate::permissions::{
    BILLING_READ, PLANS_MANAGE, PLANS_READ, SUBSCRIPTIONS_MANAGE, SUBSCRIPTIONS_READ,
};
use crate::service::{
    AssignInput, OverrideInput, PlanInput, PlanRow, PlansService, PriceInput, SubscriptionFilter,
    SubscriptionRow, now, subscription_not_found,
};

pub(crate) fn routes(service: Arc<PlansService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(catalog))
        .routes(routes!(list_plans, create_plan))
        .routes(routes!(get_plan, replace_plan))
        .routes(routes!(retire_plan))
        .routes(routes!(add_price))
        .routes(routes!(deactivate_price))
        .routes(routes!(entitlements))
        .routes(routes!(own_subscription))
        .routes(routes!(own_usage))
        .routes(routes!(list_subscriptions))
        .routes(routes!(tenant_subscription, assign_subscription))
        .routes(routes!(list_overrides, set_override))
        .routes(routes!(remove_override))
        .routes(routes!(tenant_usage))
        .merge(crate::http_lifecycle::routes())
        .with_state(service)
}

/// Nhân sự nền tảng có `permission`.
pub(crate) fn operator(principal: &Principal, permission: &Permission) -> Result<(), Error> {
    require_platform(principal)?;
    principal.require(permission)
}

// ---- Kiểu dữ liệu ---------------------------------------------------------------------------

/// A price of a plan. Prices never change: a new price is a new row, the old one is deactivated;
/// subscriptions keep their price until they change plan.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PriceResponse {
    id: Uuid,
    /// `recurring` (charged each period) or `metered` (charged for use of `limit_key`).
    #[schema(example = "recurring")]
    kind: String,
    /// `month` or `year`.
    #[schema(example = "month")]
    interval: String,
    /// How many `interval`s a period lasts, 1 to 12.
    interval_count: i32,
    /// Amount per period (`recurring`) or per unit above `included` (`metered`).
    amount: Money,
    /// The metered limit this price charges for.
    limit_key: Option<String>,
    /// Use included each period before a metered price applies.
    included: i64,
    /// Tax category of the price, when set.
    tax_category: Option<String>,
    /// `false`: no longer sold at this price.
    active: bool,
    created_at: DateTime<Utc>,
}

impl From<price::Model> for PriceResponse {
    fn from(row: price::Model) -> Self {
        // Mã tiền tệ trong database luôn hợp lệ (đã kiểm lúc ghi); lỗi thì để 0 VND cho khỏi 500.
        let amount = Money::from_columns(row.amount_minor, &row.amount_currency)
            .unwrap_or(Money::zero(autoxide_kernel::money::Currency::VND));
        PriceResponse {
            id: row.id,
            kind: row.kind,
            interval: row.interval,
            interval_count: row.interval_count,
            amount,
            limit_key: row.limit_key,
            included: row.included,
            tax_category: row.tax_category,
            active: row.active,
            created_at: row.created_at,
        }
    }
}

/// A plan: the modules, features and limits a tenant gets, and its prices.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PlanResponse {
    id: Uuid,
    #[schema(example = "pro")]
    code: String,
    #[schema(example = "Pro")]
    name: String,
    description: String,
    /// `base` (a tenant has one) or `addon` (added on top, with a quantity).
    kind: String,
    /// `draft`, `active` (sold) or `retired` (no longer sold; tenants on it keep it).
    status: String,
    /// Tenants without a subscription get this plan. At most one plan is the default; with none,
    /// such tenants are not limited.
    is_default: bool,
    /// Days of free trial for a tenant that never had one; 0 for none.
    trial_days: i32,
    /// Days a tenant keeps full access after a period ends unpaid.
    grace_days: i32,
    /// Days a suspended (read-only) subscription waits before it is canceled.
    suspend_days: i32,
    /// Days the data of a canceled subscription is kept before the tenant is deleted.
    retention_days: i32,
    /// Days before a period ends that the next one is billed and a reminder sent.
    renewal_lead_days: i32,
    /// Where a new tenant on this plan keeps its data: `shared` or `dedicated`.
    placement: String,
    /// A new tenant on this plan must pass identity checks first.
    requires_kyc: bool,
    /// A new tenant on this plan starts with sample data.
    sample_data: bool,
    /// Modules turned on (modules that are always on are not listed).
    modules: Vec<String>,
    /// Features turned on.
    features: Vec<String>,
    /// Limits by key; `null` means unlimited. A limit not listed is 0.
    limits: BTreeMap<String, Option<i64>>,
    prices: Vec<PriceResponse>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<PlanRow> for PlanResponse {
    fn from(row: PlanRow) -> Self {
        let plan = row.plan;
        PlanResponse {
            id: plan.id,
            code: plan.code,
            name: plan.name,
            description: plan.description,
            kind: plan.kind,
            status: plan.status,
            is_default: plan.is_default,
            trial_days: plan.trial_days,
            grace_days: plan.grace_days,
            suspend_days: plan.suspend_days,
            retention_days: plan.retention_days,
            renewal_lead_days: plan.renewal_lead_days,
            placement: plan.placement,
            requires_kyc: plan.requires_kyc,
            sample_data: plan.sample_data,
            modules: row.grants.modules.into_iter().collect(),
            features: row.grants.features.into_iter().collect(),
            limits: row.grants.limits,
            prices: row.prices.into_iter().map(PriceResponse::from).collect(),
            created_at: plan.created_at,
            updated_at: plan.updated_at,
        }
    }
}

fn default_kind() -> String {
    "base".to_owned()
}
fn default_status() -> String {
    "draft".to_owned()
}
fn default_placement() -> String {
    "shared".to_owned()
}
const fn default_grace() -> i32 {
    7
}
const fn default_suspend() -> i32 {
    30
}
const fn default_retention() -> i32 {
    90
}
const fn default_lead() -> i32 {
    7
}

/// A plan to create, or the whole new content of a plan.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PlanRequest {
    /// Unique code, `[a-z][a-z0-9_-]{0,63}`.
    #[schema(example = "pro")]
    code: String,
    /// Name shown to customers, 1 to 200 characters.
    #[schema(example = "Pro")]
    name: String,
    /// Up to 5000 characters.
    #[serde(default)]
    description: String,
    /// `base` (default) or `addon`.
    #[serde(default = "default_kind")]
    kind: String,
    /// `draft` (default) or `active`. Retire a plan with `POST /plans/{id}/retire`.
    #[serde(default = "default_status")]
    status: String,
    /// Make this the default plan (an active base plan); the previous default stops being one.
    #[serde(default)]
    is_default: bool,
    /// 0 to 3650; default 0.
    #[serde(default)]
    trial_days: i32,
    /// 0 to 3650; default 7.
    #[serde(default = "default_grace")]
    grace_days: i32,
    /// 0 to 3650; default 30.
    #[serde(default = "default_suspend")]
    suspend_days: i32,
    /// 0 to 3650; default 90.
    #[serde(default = "default_retention")]
    retention_days: i32,
    /// 0 to 3650; default 7.
    #[serde(default = "default_lead")]
    renewal_lead_days: i32,
    /// `shared` (default) or `dedicated`.
    #[serde(default = "default_placement")]
    placement: String,
    #[serde(default)]
    requires_kyc: bool,
    #[serde(default)]
    sample_data: bool,
    /// Modules to turn on, by name (`GET /api/v1/kernel/modules`). A base plan must also turn on
    /// the modules they need (`requires`).
    #[serde(default)]
    modules: Vec<String>,
    /// Features to turn on, by key (`features` of the modules).
    #[serde(default)]
    features: Vec<String>,
    /// Limits by key (`limits` of the modules); `null` is unlimited, a limit left out is 0.
    #[serde(default)]
    limits: BTreeMap<String, Option<i64>>,
}

impl From<PlanRequest> for PlanInput {
    fn from(input: PlanRequest) -> Self {
        PlanInput {
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
            modules: input.modules,
            features: input.features,
            limits: input.limits,
        }
    }
}

const fn one() -> i32 {
    1
}

/// A new price of a plan.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PriceRequest {
    /// `recurring` or `metered`.
    kind: String,
    /// `month` or `year`.
    interval: String,
    /// 1 to 12; default 1.
    #[serde(default = "one")]
    interval_count: i32,
    /// Amount per period (`recurring`) or per unit above `included` (`metered`); not negative.
    amount: Money,
    /// With `metered`: the metered limit charged for, e.g. `kernel.api_requests`.
    limit_key: Option<String>,
    /// With `metered`: use included each period; default 0.
    #[serde(default)]
    included: i64,
    /// Tax category, when known.
    tax_category: Option<String>,
}

impl From<PriceRequest> for PriceInput {
    fn from(input: PriceRequest) -> Self {
        PriceInput {
            kind: input.kind,
            interval: input.interval,
            interval_count: input.interval_count,
            amount: input.amount,
            limit_key: input.limit_key,
            included: input.included,
            tax_category: input.tax_category,
        }
    }
}

/// Use of one limit.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct LimitUse {
    #[schema(example = "organizations.units")]
    key: String,
    /// `count`, `amount` or `metered`.
    kind: String,
    /// The most the tenant may use; `null` is unlimited.
    max: Option<i64>,
    /// In use now (for `metered`: this month).
    used: i64,
    /// `true` when more is in use than allowed (after moving to a smaller plan): nothing is
    /// removed, but nothing new can be added.
    over_limit: bool,
}

/// What the tenant may use now. User interfaces use it to hide what the plan does not include.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct EntitlementsResponse {
    /// `true` when nothing is limited (no plan applies to this tenant).
    unrestricted: bool,
    /// `full`, or `read_only` while the subscription is suspended or canceled.
    #[schema(example = "full")]
    access: String,
    /// Modules turned on, including those always on.
    modules: Vec<String>,
    /// Features turned on.
    features: Vec<String>,
    /// Every limit a module declares, with its use.
    limits: Vec<LimitUse>,
}

/// An add-on of a subscription.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Validate)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddonItem {
    /// An active plan of kind `addon`.
    pub(crate) plan_id: Uuid,
    /// 1 to 1000; the add-on's limits count this many times.
    pub(crate) quantity: i32,
}

/// The subscription of a tenant.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = PlanSubscription)]
pub(crate) struct SubscriptionResponse {
    id: Uuid,
    tenant_id: Uuid,
    plan_id: Uuid,
    price_id: Option<Uuid>,
    /// `trialing`, `active`, `past_due` (unpaid, in the grace period), `suspended` (read-only),
    /// `canceled` (read-only, data kept until `delete_after`) or `deleted`.
    #[schema(example = "active")]
    status: String,
    current_period_start: DateTime<Utc>,
    current_period_end: DateTime<Utc>,
    trial_ends_at: Option<DateTime<Utc>>,
    grace_ends_at: Option<DateTime<Utc>>,
    suspended_at: Option<DateTime<Utc>>,
    canceled_at: Option<DateTime<Utc>>,
    delete_after: Option<DateTime<Utc>>,
    /// Cancels when the current period ends.
    cancel_at_period_end: bool,
    /// Plan the subscription moves to when the period ends (after a downgrade).
    scheduled_plan_id: Option<Uuid>,
    scheduled_price_id: Option<Uuid>,
    /// The tenant already had its free trial.
    trial_used: bool,
    addons: Vec<AddonItem>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<SubscriptionRow> for SubscriptionResponse {
    fn from(row: SubscriptionRow) -> Self {
        let sub = row.subscription;
        SubscriptionResponse {
            id: sub.id,
            tenant_id: sub.tenant_id,
            plan_id: sub.plan_id,
            price_id: sub.price_id,
            status: sub.status,
            current_period_start: sub.current_period_start,
            current_period_end: sub.current_period_end,
            trial_ends_at: sub.trial_ends_at,
            grace_ends_at: sub.grace_ends_at,
            suspended_at: sub.suspended_at,
            canceled_at: sub.canceled_at,
            delete_after: sub.delete_after,
            cancel_at_period_end: sub.cancel_at_period_end,
            scheduled_plan_id: sub.scheduled_plan_id,
            scheduled_price_id: sub.scheduled_price_id,
            trial_used: sub.trial_used,
            addons: row
                .addons
                .into_iter()
                .map(|addon| AddonItem { plan_id: addon.plan_id, quantity: addon.quantity })
                .collect(),
            created_at: sub.created_at,
            updated_at: sub.updated_at,
        }
    }
}

const fn yes() -> bool {
    true
}

/// Put a tenant on a plan.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssignRequest {
    /// An active base plan.
    plan_id: Uuid,
    /// An active recurring price of that plan; without it, periods last a month and nothing is
    /// charged.
    price_id: Option<Uuid>,
    /// Start with the plan's free trial when the tenant never had one; default `true`.
    #[serde(default = "yes")]
    trial: bool,
    /// Replace the add-ons with these; leave out to keep them.
    addons: Option<Vec<AddonItem>>,
}

impl From<AssignRequest> for AssignInput {
    fn from(input: AssignRequest) -> Self {
        AssignInput {
            plan_id: input.plan_id,
            price_id: input.price_id,
            trial: input.trial,
            addons: input
                .addons
                .map(|items| items.into_iter().map(|item| (item.plan_id, item.quantity)).collect()),
        }
    }
}

/// An override for one tenant: turn a module or feature on or off, or replace a limit.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct OverrideResponse {
    id: Uuid,
    /// `module`, `feature` or `limit`.
    kind: String,
    key: String,
    /// With `module` / `feature`.
    enabled: Option<bool>,
    /// With `limit`: the new maximum; `null` is unlimited.
    max_value: Option<i64>,
    reason: String,
    /// The override no longer applies from this time.
    expires_at: Option<DateTime<Utc>>,
    /// User who set it.
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
}

impl From<grant::Model> for OverrideResponse {
    fn from(row: grant::Model) -> Self {
        OverrideResponse {
            id: row.id,
            kind: row.kind,
            key: row.key,
            enabled: row.enabled,
            max_value: row.max_value,
            reason: row.reason,
            expires_at: row.expires_at,
            created_by: row.created_by,
            created_at: row.created_at,
        }
    }
}

/// An override to set; it replaces one of the same kind and key.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct OverrideRequest {
    /// `module`, `feature` or `limit`.
    kind: String,
    /// Module name, feature key or limit key.
    key: String,
    /// Required with `module` / `feature`.
    enabled: Option<bool>,
    /// With `limit`: the new maximum; leave out or `null` for unlimited.
    max_value: Option<i64>,
    /// Why, 1 to 500 characters (kept for the audit trail).
    reason: String,
    /// When the override stops applying.
    expires_at: Option<DateTime<Utc>>,
}

impl From<OverrideRequest> for OverrideInput {
    fn from(input: OverrideRequest) -> Self {
        OverrideInput {
            kind: input.kind,
            key: input.key,
            enabled: input.enabled,
            max_value: input.max_value,
            reason: input.reason,
            expires_at: input.expires_at,
        }
    }
}

/// Filters of the subscription list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SubscriptionsQuery {
    /// Only subscriptions with this status.
    status: Option<String>,
    /// Only subscriptions on this plan.
    plan_id: Option<Uuid>,
}

// ---- Dựng câu trả lời quyền dùng ------------------------------------------------------------

pub(crate) async fn describe_entitlements(
    service: &PlansService,
    tenant: TenantId,
) -> Result<EntitlementsResponse, Error> {
    let snapshot = service.snapshot(tenant).await?;
    let limits = limit_uses(service, tenant).await?;
    let catalog = service.entitlements.catalog();
    let modules = catalog
        .modules()
        .iter()
        .filter(|module| catalog.is_always_enabled(module) || snapshot.has_module(module))
        .map(|module| (*module).to_owned())
        .collect();
    let features = if snapshot.unrestricted {
        catalog.features().map(|found| found.feature.key().to_owned()).collect()
    } else {
        snapshot.features.iter().cloned().collect()
    };
    Ok(EntitlementsResponse {
        unrestricted: snapshot.unrestricted,
        access: match snapshot.access {
            TenantAccess::Full => "full",
            TenantAccess::ReadOnly => "read_only",
        }
        .to_owned(),
        modules,
        features,
        limits,
    })
}

pub(crate) async fn limit_uses(
    service: &PlansService,
    tenant: TenantId,
) -> Result<Vec<LimitUse>, Error> {
    let snapshot = service.snapshot(tenant).await?;
    let used = service.usage(tenant, now()).await?;
    Ok(service
        .entitlements
        .catalog()
        .limits()
        .map(|found| {
            let key = found.limit.key();
            let max = snapshot.limit(key);
            let used = used.get(key).copied().unwrap_or(0);
            LimitUse {
                key: key.to_owned(),
                kind: found.limit.kind().as_str().to_owned(),
                max,
                used,
                over_limit: max.is_some_and(|max| used > max),
            }
        })
        .collect())
}

// ---- Handler --------------------------------------------------------------------------------

/// List the plans on sale, with their prices.
///
/// No sign-in needed: a sign-up page shows these. Only active plans and active prices.
#[utoipa::path(
    get,
    path = "/catalog",
    extensions(("x-public" = json!(true))),
    operation_id = "plans_catalog",
    responses((status = 200, description = "Active base plans and add-ons, by code.", body = Data<Vec<PlanResponse>>))
)]
async fn catalog(
    State(service): State<Arc<PlansService>>,
) -> Result<Data<Vec<PlanResponse>>, Error> {
    Ok(Data { data: service.catalog().await?.into_iter().map(PlanResponse::from).collect() })
}

/// List every plan, including drafts and retired plans.
///
/// Platform staff with `plans.plans.read`.
#[utoipa::path(
    get,
    path = "/plans",
    extensions(("x-permissions" = json!(["plans.plans.read"]))),
    operation_id = "list_plans",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of plans, newest first.", body = Page<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.read` (`permission_denied`).", body = Problem),
    )
)]
async fn list_plans(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<PlanResponse>, Error> {
    operator(&principal, &PLANS_READ)?;
    Ok(service.list_plans(&PageRequest::try_from(page)?).await?.map(PlanResponse::from))
}

/// Create a plan.
///
/// Platform staff with `plans.plans.manage`. Modules, features and limits must be ones the
/// installed modules declare.
#[utoipa::path(
    post,
    path = "/plans",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "create_plan",
    security(("bearer" = [])),
    request_body = PlanRequest,
    responses(
        (status = 201, description = "Created.", body = Data<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 409, description = "Another plan has this code; code `plans.plan_code_taken`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`; an unknown module, feature or limit is `unknown`), or a module lacks a module it needs (`plans.module_dependency_missing`).", body = Problem),
    )
)]
async fn create_plan(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    ValidJson(input): ValidJson<PlanRequest>,
) -> Result<Created<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    let row = service.create_plan(Actor::from(&principal), input.into()).await?;
    Ok(Created::new(format!("/plans/plans/{}", row.plan.id), row.into()))
}

/// Get a plan.
///
/// Platform staff with `plans.plans.read`.
#[utoipa::path(
    get,
    path = "/plans/{id}",
    extensions(("x-permissions" = json!(["plans.plans.read"]))),
    operation_id = "get_plan",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the plan.")),
    responses(
        (status = 200, description = "The plan.", body = Data<PlanResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.read` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such plan; code `plans.plan_not_found`.", body = Problem),
    )
)]
async fn get_plan(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<PlanResponse>, Error> {
    operator(&principal, &PLANS_READ)?;
    let row = service.get_plan(id).await?;
    Ok(Tagged { version: row.plan.updated_at, data: row.into() })
}

/// Replace a plan.
///
/// Platform staff with `plans.plans.manage`. The body is the whole plan; tenants on it get the new
/// modules, features and limits within `AUTOXIDE_PLANS_CACHE_SECONDS`.
#[utoipa::path(
    put,
    path = "/plans/{id}",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "replace_plan",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the plan."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the plan since; otherwise `412`.")),
    request_body = PlanRequest,
    responses(
        (status = 200, description = "The plan after the change.", body = Data<PlanResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such plan; code `plans.plan_not_found`.", body = Problem),
        (status = 409, description = "Another plan has this code (`plans.plan_code_taken`), the plan is retired (`plans.plan_not_active`), or tenants use it and `kind` would change (`plans.plan_in_use`).", body = Problem),
        (status = 412, description = "`If-Match` does not match: get the plan again and retry; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or a module lacks a module it needs (`plans.module_dependency_missing`).", body = Problem),
    )
)]
async fn replace_plan(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<PlanRequest>,
) -> Result<Tagged<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    let expected = if_match.expected()?;
    let row = service.update_plan(Actor::from(&principal), id, input.into(), expected).await?;
    Ok(Tagged { version: row.plan.updated_at, data: row.into() })
}

/// Retire a plan.
///
/// Platform staff with `plans.plans.manage`. The plan is no longer sold or the default; tenants on
/// it keep it. A plan is never deleted.
#[utoipa::path(
    post,
    path = "/plans/{id}/retire",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "retire_plan",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the plan.")),
    responses(
        (status = 200, description = "The plan, now `retired`.", body = Data<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such plan; code `plans.plan_not_found`.", body = Problem),
    )
)]
async fn retire_plan(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    Ok(Data { data: service.retire_plan(Actor::from(&principal), id).await?.into() })
}

/// Add a price to a plan.
///
/// Platform staff with `plans.plans.manage`. To change a price, add a new one and deactivate the old.
#[utoipa::path(
    post,
    path = "/plans/{id}/prices",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "add_plan_price",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the plan.")),
    request_body = PriceRequest,
    responses(
        (status = 201, description = "The plan with the new price.", body = Data<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such plan; code `plans.plan_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn add_price(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<PriceRequest>,
) -> Result<Created<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    let row = service.add_price(Actor::from(&principal), id, input.into()).await?;
    Ok(Created::new(format!("/plans/plans/{id}"), row.into()))
}

/// Deactivate a price.
///
/// Platform staff with `plans.plans.manage`. Subscriptions at this price keep it until they change
/// plan.
#[utoipa::path(
    post,
    path = "/prices/{id}/deactivate",
    extensions(("x-permissions" = json!(["plans.plans.manage"]))),
    operation_id = "deactivate_plan_price",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the price.")),
    responses(
        (status = 200, description = "The plan of the price.", body = Data<PlanResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.plans.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such price; code `plans.price_not_found`.", body = Problem),
    )
)]
async fn deactivate_price(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PlanResponse>, Error> {
    operator(&principal, &PLANS_MANAGE)?;
    Ok(Data { data: service.deactivate_price(Actor::from(&principal), id).await?.into() })
}

/// Get what your tenant may use now.
///
/// Any signed-in user. Modules, features, limits with their use, and whether the data is
/// read-only. User interfaces use it to hide what the plan does not include.
#[utoipa::path(
    get,
    path = "/entitlements",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_entitlements",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "What the tenant may use now.", body = Data<EntitlementsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn entitlements(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    _principal: Principal,
) -> Result<Data<EntitlementsResponse>, Error> {
    Ok(Data { data: describe_entitlements(&service, tenant.id()).await? })
}

/// Get your tenant's subscription.
///
/// Requires `plans.billing.read`.
#[utoipa::path(
    get,
    path = "/subscription",
    extensions(("x-permissions" = json!(["plans.billing.read"]))),
    operation_id = "get_own_subscription",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The subscription.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The tenant has no subscription; code `plans.subscription_not_found`.", body = Problem),
    )
)]
async fn own_subscription(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<SubscriptionResponse>, Error> {
    principal.require(&BILLING_READ)?;
    let row = service.subscription(tenant.id()).await?.ok_or_else(subscription_not_found)?;
    Ok(Data { data: row.into() })
}

/// Get your tenant's use of every limit.
///
/// Requires `plans.billing.read`.
#[utoipa::path(
    get,
    path = "/usage",
    extensions(("x-permissions" = json!(["plans.billing.read"]))),
    operation_id = "get_own_usage",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Use of each limit (metered limits: this month).", body = Data<Vec<LimitUse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `plans.billing.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn own_usage(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<LimitUse>>, Error> {
    principal.require(&BILLING_READ)?;
    Ok(Data { data: limit_uses(&service, tenant.id()).await? })
}

/// List the subscriptions of every tenant.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/subscriptions",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "list_subscriptions",
    security(("bearer" = [])),
    params(SubscriptionsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of subscriptions, newest first (without add-ons).", body = Page<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
    )
)]
async fn list_subscriptions(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Query(filter): Query<SubscriptionsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<SubscriptionResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let filter = SubscriptionFilter { status: filter.status, plan_id: filter.plan_id };
    let page = service.list_subscriptions(&filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(|subscription| SubscriptionRow { subscription, addons: Vec::new() }.into()))
}

/// Get a tenant's subscription.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/subscription",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "get_tenant_subscription",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    responses(
        (status = 200, description = "The subscription.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 404, description = "The tenant has no subscription; code `plans.subscription_not_found`.", body = Problem),
    )
)]
async fn tenant_subscription(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
) -> Result<Data<SubscriptionResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let row = service
        .subscription(TenantId::from_uuid(tenant_id))
        .await?
        .ok_or_else(subscription_not_found)?;
    Ok(Data { data: row.into() })
}

/// Put a tenant on a plan.
///
/// Platform staff with `plans.subscriptions.manage`. The first time this creates the
/// subscription (with the plan's trial when the tenant never had one); afterwards it moves the
/// tenant to the plan at once. A canceled subscription starts again.
#[utoipa::path(
    put,
    path = "/tenants/{tenant_id}/subscription",
    extensions(("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "assign_subscription",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    request_body = AssignRequest,
    responses(
        (status = 200, description = "The subscription after the change.", body = Data<SubscriptionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such tenant (`tenant_not_found`), plan (`plans.plan_not_found`) or price (`plans.price_not_found`).", body = Problem),
        (status = 409, description = "The plan is not sold (`plans.plan_not_active`) or the subscription is deleted (`plans.invalid_transition`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), a plan is of the wrong kind (`plans.addon_not_allowed`), or the tenant is the platform (`plans.platform_exempt`).", body = Problem),
    )
)]
async fn assign_subscription(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
    ValidJson(input): ValidJson<AssignRequest>,
) -> Result<Data<SubscriptionResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    let row = service
        .assign(Actor::from(&principal), TenantId::from_uuid(tenant_id), input.into())
        .await?;
    Ok(Data { data: row.into() })
}

/// List a tenant's overrides.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/overrides",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "list_overrides",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    responses(
        (status = 200, description = "The overrides, oldest first (expired ones included).", body = Data<Vec<OverrideResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
    )
)]
async fn list_overrides(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
) -> Result<Data<Vec<OverrideResponse>>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let rows = service.overrides(TenantId::from_uuid(tenant_id)).await?;
    Ok(Data { data: rows.into_iter().map(OverrideResponse::from).collect() })
}

/// Set an override for a tenant.
///
/// Platform staff with `plans.subscriptions.manage`. Replaces an override of the same kind and key.
#[utoipa::path(
    post,
    path = "/tenants/{tenant_id}/overrides",
    extensions(("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "set_override",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    request_body = OverrideRequest,
    responses(
        (status = 201, description = "Created.", body = Data<OverrideResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or the tenant is the platform (`plans.platform_exempt`).", body = Problem),
    )
)]
async fn set_override(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
    ValidJson(input): ValidJson<OverrideRequest>,
) -> Result<Created<OverrideResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    let row =
        service.set_override(&principal, TenantId::from_uuid(tenant_id), input.into()).await?;
    Ok(Created::new(format!("/plans/tenants/{tenant_id}/overrides/{}", row.id), row.into()))
}

/// Remove an override of a tenant.
///
/// Platform staff with `plans.subscriptions.manage`.
#[utoipa::path(
    delete,
    path = "/tenants/{tenant_id}/overrides/{id}",
    extensions(("x-permissions" = json!(["plans.subscriptions.manage"]))),
    operation_id = "remove_override",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant."), ("id" = Uuid, Path, description = "Id of the override.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.manage` (`permission_denied`).", body = Problem),
        (status = 404, description = "No such override; code `plans.override_not_found`.", body = Problem),
    )
)]
async fn remove_override(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path((tenant_id, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Error> {
    operator(&principal, &SUBSCRIPTIONS_MANAGE)?;
    service.remove_override(Actor::from(&principal), TenantId::from_uuid(tenant_id), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get a tenant's use of every limit.
///
/// Platform staff with `plans.subscriptions.read`.
#[utoipa::path(
    get,
    path = "/tenants/{tenant_id}/usage",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "get_tenant_usage",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant.")),
    responses(
        (status = 200, description = "Use of each limit (metered limits: this month).", body = Data<Vec<LimitUse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
    )
)]
async fn tenant_usage(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Path(tenant_id): Path<Uuid>,
) -> Result<Data<Vec<LimitUse>>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    Ok(Data { data: limit_uses(&service, TenantId::from_uuid(tenant_id)).await? })
}
