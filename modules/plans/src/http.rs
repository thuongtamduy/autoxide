//! Route REST của module `plans`, dưới `/api/v1/plans`.
//!
//! "Vận hành" (gói, thuê bao của mọi khách) cần permission giữ ở khách nền tảng; "khách"
//! (`billing`) là permission ở khách hiện tại.

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::entitlement::TenantAccess;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest, page_of};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{TenantContext, TenantId, require_platform};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
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
    AssignInput, OverrideFilter, OverrideInput, PlanInput, PlanRow, PlansService, PriceInput,
    SubscriptionFilter, SubscriptionRow, now, subscription_not_found,
};

mod overrides;
mod plans;
mod subscriptions;
mod usage;
pub(crate) use overrides::*;
pub(crate) use plans::*;
pub(crate) use subscriptions::*;
pub(crate) use usage::*;

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
        .routes(routes!(all_overrides))
        .routes(routes!(tenant_entitlements))
        .routes(routes!(list_overrides, set_override))
        .routes(routes!(remove_override))
        .routes(routes!(tenant_usage))
        .merge(crate::http_lifecycle::routes())
        .merge(crate::billing_profile::routes())
        .merge(crate::provisioning::routes())
        .merge(crate::signup_http::routes())
        .merge(crate::store_http::routes())
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
    created_at: Timestamp,
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
    /// New tenants may choose this plan when they sign up by themselves (`POST /signups`).
    self_signup: bool,
    /// Modules turned on (modules that are always on are not listed).
    modules: Vec<String>,
    /// Features turned on.
    features: Vec<String>,
    /// Limits by key; `null` means unlimited. A limit not listed is 0.
    limits: BTreeMap<String, Option<i64>>,
    prices: Vec<PriceResponse>,
    /// Tenants on the plan now (subscriptions not canceled, as base plan or add-on). Only for
    /// platform staff: `null` in the public catalog.
    subscriber_count: Option<i64>,
    created_at: Timestamp,
    updated_at: Timestamp,
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
            self_signup: plan.self_signup,
            modules: row.grants.modules.into_iter().collect(),
            features: row.grants.features.into_iter().collect(),
            limits: row.grants.limits,
            prices: row.prices.into_iter().map(PriceResponse::from).collect(),
            subscriber_count: row.subscribers,
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
    /// A new tenant must pass identity checks first (only through `POST /signups`).
    #[serde(default)]
    requires_kyc: bool,
    #[serde(default)]
    sample_data: bool,
    /// New tenants may choose this plan when they sign up by themselves (an active base plan;
    /// sign-up must be turned on with `AUTOXIDE_PLANS_SIGNUP`). Default `false`.
    #[serde(default)]
    self_signup: bool,
    /// Modules to turn on, by name (`GET /api/v1/kernel/modules`). A base plan must also turn on
    /// the modules they need (`requires`).
    #[serde(default)]
    modules: Vec<String>,
    /// Features to turn on, by key (`features` of the modules).
    #[serde(default)]
    features: Vec<String>,
    /// Limits by key (`limits` of the modules); `null` is unlimited, a limit left out is 0. A limit
    /// that has parts also takes a cap per part, `<limit>.<part>` (for example
    /// `organizations.units.store`); a part left out has no cap of its own.
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
            self_signup: input.self_signup,
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
    /// The limit; `<limit>.<part>` is the separate cap of one part of a limit that has parts (for
    /// example `organizations.units.store`: units of type `store`), listed when the plan caps that
    /// part or the tenant uses it.
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
    /// Code of the plan (also when it is no longer sold).
    plan_code: Option<String>,
    /// Name of the plan (also when it is no longer sold).
    plan_name: Option<String>,
    price_id: Option<Uuid>,
    /// `trialing`, `active`, `past_due` (unpaid, in the grace period), `suspended` (read-only),
    /// `canceled` (read-only, data kept until `delete_after`) or `deleted`.
    #[schema(example = "active")]
    status: String,
    current_period_start: Timestamp,
    current_period_end: Timestamp,
    trial_ends_at: Option<Timestamp>,
    grace_ends_at: Option<Timestamp>,
    suspended_at: Option<Timestamp>,
    canceled_at: Option<Timestamp>,
    delete_after: Option<Timestamp>,
    /// Cancels when the current period ends.
    cancel_at_period_end: bool,
    /// Plan the subscription moves to when the period ends (after a downgrade).
    scheduled_plan_id: Option<Uuid>,
    scheduled_price_id: Option<Uuid>,
    /// The tenant already had its free trial.
    trial_used: bool,
    addons: Vec<AddonItem>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<SubscriptionRow> for SubscriptionResponse {
    fn from(row: SubscriptionRow) -> Self {
        let sub = row.subscription;
        let (plan_code, plan_name) =
            row.plan.map_or((None, None), |(code, name)| (Some(code), Some(name)));
        SubscriptionResponse {
            id: sub.id,
            tenant_id: sub.tenant_id,
            plan_id: sub.plan_id,
            plan_code,
            plan_name,
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
    /// The tenant it applies to.
    tenant_id: Uuid,
    /// `module`, `feature` or `limit`.
    kind: String,
    key: String,
    /// With `module` / `feature`.
    enabled: Option<bool>,
    /// With `limit`: the new maximum; `null` is unlimited.
    max_value: Option<i64>,
    reason: String,
    /// The override no longer applies from this time.
    expires_at: Option<Timestamp>,
    /// User who set it.
    created_by: Option<Uuid>,
    created_at: Timestamp,
}

impl From<grant::Model> for OverrideResponse {
    fn from(row: grant::Model) -> Self {
        OverrideResponse {
            id: row.id,
            tenant_id: row.tenant_id,
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
    expires_at: Option<Timestamp>,
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
    /// Only the subscription of this tenant.
    tenant_id: Option<Uuid>,
}

/// Filters of the list of every tenant's overrides.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct OverridesQuery {
    /// Only the overrides of this tenant.
    tenant_id: Option<Uuid>,
    /// Only `module`, `feature` or `limit` overrides.
    kind: Option<String>,
    /// Only overrides of this module, feature or limit key.
    key: Option<String>,
    /// `true`: only overrides that still apply; `false`: only expired ones.
    active: Option<bool>,
}

// ---- Dựng câu trả lời quyền dùng ------------------------------------------------------------

// ---- Handler --------------------------------------------------------------------------------
