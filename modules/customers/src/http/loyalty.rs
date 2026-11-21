//! Endpoint điểm thưởng: quy tắc, hạng, chương trình voucher (quản trị); điểm của một khách và điều
//! chỉnh tay (nhân viên); điểm của chính mình và đổi điểm lấy voucher (người mua).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::views::try_page;
use crate::entity::{loyalty_rule, points_entry, points_lot, tier, voucher_program};
use crate::loyalty::admin::{NewProgram, NewRule, NewTier, PointsView};
use crate::permissions::{LOYALTY_ADJUST, LOYALTY_MANAGE, READ};
use crate::service::CustomersService;

/// A loyalty rule: how points are earned and redeemed from `effective_from` on.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct LoyaltyRule {
    pub id: Uuid,
    /// Paid for goods per earning unit (smallest currency unit), e.g. `10000`.
    pub earn_per_minor: i64,
    /// Points per earning unit.
    pub points_per_unit: i64,
    /// What one point is worth when redeemed.
    pub point_value: Money,
    pub points_valid_months: i32,
    pub min_redeem_points: i64,
    /// Points pay at most this share of an order, in basis points (`5000` = 50%).
    pub max_redeem_ratio_bp: i32,
    pub effective_from: Timestamp,
}

impl LoyaltyRule {
    fn new(row: loyalty_rule::Model) -> Result<Self, Error> {
        Ok(LoyaltyRule {
            point_value: Money::from_columns(row.point_value_minor, &row.currency)?,
            id: row.id,
            earn_per_minor: row.earn_per_minor,
            points_per_unit: row.points_per_unit,
            points_valid_months: row.points_valid_months,
            min_redeem_points: row.min_redeem_points,
            max_redeem_ratio_bp: row.max_redeem_ratio_bp,
            effective_from: row.effective_from,
        })
    }
}

/// A new loyalty rule; it replaces the current one from `effective_from` on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateLoyaltyRuleRequest {
    earn_per_minor: i64,
    points_per_unit: i64,
    /// Smallest currency unit, e.g. `1000` (1,000 VND per point).
    point_value_minor: i64,
    /// ISO 4217 code, e.g. `VND`.
    currency: String,
    /// 1 to 120.
    points_valid_months: i32,
    #[serde(default)]
    min_redeem_points: i64,
    /// 1 to 10000.
    max_redeem_ratio_bp: i32,
    /// Omitted: now. Never in the past.
    effective_from: Option<Timestamp>,
}

/// A membership tier version.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct MembershipTier {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    /// Spent on goods over the last 12 months to reach the tier.
    pub threshold: Money,
    /// Earning multiplier in basis points (`12000` = ×1.2).
    pub earn_multiplier_bp: i32,
    /// Higher wins when several tiers are reached.
    pub rank: i32,
    pub effective_from: Timestamp,
}

impl MembershipTier {
    fn new(row: tier::Model) -> Result<Self, Error> {
        Ok(MembershipTier {
            threshold: Money::from_columns(row.threshold_minor, &row.currency)?,
            id: row.id,
            code: row.code,
            name: row.name,
            earn_multiplier_bp: row.earn_multiplier_bp,
            rank: row.rank,
            effective_from: row.effective_from,
        })
    }
}

/// A new membership tier, or a new version of one (same `code`).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateMembershipTierRequest {
    /// `a-z`, `0-9`, `_`, e.g. `gold`.
    code: String,
    name: String,
    threshold_minor: i64,
    currency: String,
    /// `10000` = ×1.
    earn_multiplier_bp: i32,
    rank: i32,
    effective_from: Option<Timestamp>,
}

/// A voucher reward: a voucher promotion given on birthdays, after a purchase or for points.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct VoucherReward {
    pub id: Uuid,
    /// `birthday`, `purchase` or `points`.
    pub kind: String,
    /// The voucher promotion of the promotions module.
    pub promotion_id: Uuid,
    /// Points it costs (`points`).
    pub points_cost: Option<i64>,
    /// Smallest goods amount of the order (`purchase`).
    pub min_order_minor: Option<i64>,
    pub active: bool,
}

impl From<voucher_program::Model> for VoucherReward {
    fn from(row: voucher_program::Model) -> Self {
        VoucherReward {
            id: row.id,
            kind: row.kind,
            promotion_id: row.promotion_id,
            points_cost: row.points_cost,
            min_order_minor: row.min_order_minor,
            active: row.active,
        }
    }
}

/// A new voucher reward.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateVoucherRewardRequest {
    /// `birthday`, `purchase` or `points`.
    kind: String,
    /// A promotion of kind `voucher`.
    promotion_id: Uuid,
    points_cost: Option<i64>,
    min_order_minor: Option<i64>,
}

/// Turn a voucher reward on or off.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetVoucherRewardRequest {
    active: bool,
}

/// A lot of points with its own expiry.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PointsLot {
    pub id: Uuid,
    pub earned: i64,
    /// Not yet used (includes `held`).
    pub remaining: i64,
    /// Held for orders waiting for payment.
    pub held: i64,
    pub expires_at: Timestamp,
}

impl From<points_lot::Model> for PointsLot {
    fn from(row: points_lot::Model) -> Self {
        PointsLot {
            id: row.id,
            earned: row.earned,
            remaining: row.remaining,
            held: row.held,
            expires_at: row.expires_at,
        }
    }
}

/// Loyalty points of a customer, with the program rules shown to buyers.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CustomerPoints {
    /// Points that can be used now.
    pub balance: i64,
    /// Points held for orders waiting for payment.
    pub held: i64,
    /// Lots with points left, soonest expiry first.
    pub lots: Vec<PointsLot>,
    /// The rule in force; absent when the shop has no loyalty program.
    pub rule: Option<LoyaltyRule>,
}

impl CustomerPoints {
    fn new(view: PointsView) -> Result<Self, Error> {
        Ok(CustomerPoints {
            balance: view.balance,
            held: view.held,
            lots: view.lots.into_iter().map(PointsLot::from).collect(),
            rule: view.rule.map(LoyaltyRule::new).transpose()?,
        })
    }
}

/// One line of the points history.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PointsEntry {
    pub id: Uuid,
    /// `earn`, `hold`, `redeem`, `release`, `expire`, `adjust`, `reverse` or `restore`.
    pub kind: String,
    /// Positive: added; negative: removed.
    pub points: i64,
    pub reference: String,
    pub reason: Option<String>,
    pub created_at: Timestamp,
}

impl From<points_entry::Model> for PointsEntry {
    fn from(row: points_entry::Model) -> Self {
        PointsEntry {
            id: row.id,
            kind: row.kind,
            points: row.points,
            reference: row.reference,
            reason: row.reason,
            created_at: row.created_at,
        }
    }
}

/// A manual change of points.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdjustPointsRequest {
    /// Positive adds a lot under the current rule; negative removes points, oldest first.
    points: i64,
    /// Your reference (ticket, complaint…); the same reference never applies twice.
    reference: String,
    /// Why, kept in the history; 1 to 500 characters.
    reason: String,
}

/// Exchange points for a voucher.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RedeemVoucherRequest {
    /// A voucher reward of kind `points`.
    reward_id: Uuid,
}

/// The voucher put in your wallet.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct RedeemedVoucher {
    pub voucher_id: Uuid,
    pub promotion_id: Uuid,
}

/// List loyalty rules (all versions).
///
/// Requires `customers.loyalty.manage`.
#[utoipa::path(
    get, path = "/loyalty/rules",
    extensions(("x-permissions" = json!(["customers.loyalty.manage"]))),
    operation_id = "list_loyalty_rules", security(("bearer" = [])), params(PageQuery),
    responses(
        (status = 200, description = "One page of rule versions.", body = Page<LoyaltyRule>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.loyalty.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_rules(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<LoyaltyRule>, Error> {
    principal.require(&LOYALTY_MANAGE)?;
    try_page(service.rules(&tenant, &PageRequest::try_from(page)?).await?, LoyaltyRule::new)
}

/// Add a loyalty rule version.
///
/// Requires `customers.loyalty.manage` for the whole shop. Rules are dated data: a new version
/// takes effect at `effective_from` (now or later) and never changes the past.
#[utoipa::path(
    post, path = "/loyalty/rules",
    extensions(("x-permissions" = json!(["customers.loyalty.manage"]))),
    operation_id = "create_loyalty_rule", security(("bearer" = [])), request_body = CreateLoyaltyRuleRequest,
    responses(
        (status = 201, description = "Added.", body = Data<LoyaltyRule>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.loyalty.manage` for the whole shop; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or `effective_from` is in the past (`customers.rule_effective_in_past`).", body = Problem),
    )
)]
pub(crate) async fn create_rule(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateLoyaltyRuleRequest>,
) -> Result<Created<LoyaltyRule>, Error> {
    principal.require(&LOYALTY_MANAGE)?;
    let rule = NewRule {
        earn_per_minor: input.earn_per_minor,
        points_per_unit: input.points_per_unit,
        point_value_minor: input.point_value_minor,
        currency: input.currency,
        points_valid_months: input.points_valid_months,
        min_redeem_points: input.min_redeem_points,
        max_redeem_ratio_bp: input.max_redeem_ratio_bp,
        effective_from: input.effective_from,
    };
    let row = service.add_rule(&tenant, &principal, rule).await?;
    Ok(Created::new(format!("/customers/loyalty/rules/{}", row.id), LoyaltyRule::new(row)?))
}

/// List membership tiers (all versions).
///
/// Requires `customers.loyalty.manage`.
#[utoipa::path(
    get, path = "/loyalty/tiers",
    extensions(("x-permissions" = json!(["customers.loyalty.manage"]))),
    operation_id = "list_membership_tiers", security(("bearer" = [])), params(PageQuery),
    responses(
        (status = 200, description = "One page of tier versions.", body = Page<MembershipTier>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.loyalty.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_tiers(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<MembershipTier>, Error> {
    principal.require(&LOYALTY_MANAGE)?;
    try_page(service.tiers(&tenant, &PageRequest::try_from(page)?).await?, MembershipTier::new)
}

/// Add a membership tier, or a new version of one.
///
/// Requires `customers.loyalty.manage` for the whole shop. Customers move between tiers by what
/// they spent on goods over the last 12 months; checked when an order completes and every day.
#[utoipa::path(
    post, path = "/loyalty/tiers",
    extensions(("x-permissions" = json!(["customers.loyalty.manage"]))),
    operation_id = "create_membership_tier", security(("bearer" = [])), request_body = CreateMembershipTierRequest,
    responses(
        (status = 201, description = "Added.", body = Data<MembershipTier>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.loyalty.manage` for the whole shop; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 409, description = "This tier already has a version from the same moment; code `conflict`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or `effective_from` is in the past (`customers.rule_effective_in_past`).", body = Problem),
    )
)]
pub(crate) async fn create_tier(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateMembershipTierRequest>,
) -> Result<Created<MembershipTier>, Error> {
    principal.require(&LOYALTY_MANAGE)?;
    let tier = NewTier {
        code: input.code,
        name: input.name,
        threshold_minor: input.threshold_minor,
        currency: input.currency,
        earn_multiplier_bp: input.earn_multiplier_bp,
        rank: input.rank,
        effective_from: input.effective_from,
    };
    let row = service.add_tier(&tenant, &principal, tier).await?;
    Ok(Created::new(format!("/customers/loyalty/tiers/{}", row.id), MembershipTier::new(row)?))
}

/// List voucher rewards.
///
/// Requires `customers.loyalty.manage`.
#[utoipa::path(
    get, path = "/loyalty/voucher-rewards",
    extensions(("x-permissions" = json!(["customers.loyalty.manage"]))),
    operation_id = "list_voucher_rewards", security(("bearer" = [])), params(PageQuery),
    responses(
        (status = 200, description = "One page of voucher rewards.", body = Page<VoucherReward>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.loyalty.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_rewards(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<VoucherReward>, Error> {
    principal.require(&LOYALTY_MANAGE)?;
    Ok(service
        .programs_page(&tenant, &PageRequest::try_from(page)?, false)
        .await?
        .map(VoucherReward::from))
}

/// Add a voucher reward.
///
/// Requires `customers.loyalty.manage` for the whole shop. Vouchers are granted through the
/// promotions module; granting twice for the same birthday year, order or exchange gives nothing more.
#[utoipa::path(
    post, path = "/loyalty/voucher-rewards",
    extensions(("x-permissions" = json!(["customers.loyalty.manage"]))),
    operation_id = "create_voucher_reward", security(("bearer" = [])), request_body = CreateVoucherRewardRequest,
    responses(
        (status = 201, description = "Added.", body = Data<VoucherReward>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.loyalty.manage` for the whole shop; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_reward(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateVoucherRewardRequest>,
) -> Result<Created<VoucherReward>, Error> {
    principal.require(&LOYALTY_MANAGE)?;
    let program = NewProgram {
        kind: input.kind,
        promotion_id: input.promotion_id,
        points_cost: input.points_cost,
        min_order_minor: input.min_order_minor,
    };
    let row = service.add_program(&tenant, &principal, program).await?;
    Ok(Created::new(format!("/customers/loyalty/voucher-rewards/{}", row.id), row.into()))
}

/// Turn a voucher reward on or off.
///
/// Requires `customers.loyalty.manage` for the whole shop.
#[utoipa::path(
    put, path = "/loyalty/voucher-rewards/{id}",
    extensions(("x-permissions" = json!(["customers.loyalty.manage"]))),
    operation_id = "set_voucher_reward", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the voucher reward.")),
    request_body = SetVoucherRewardRequest,
    responses(
        (status = 200, description = "Changed.", body = Data<VoucherReward>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.loyalty.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No voucher reward with this id; code `customers.voucher_program_not_found`.", body = Problem),
    )
)]
pub(crate) async fn set_reward(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SetVoucherRewardRequest>,
) -> Result<Data<VoucherReward>, Error> {
    principal.require(&LOYALTY_MANAGE)?;
    Ok(Data {
        data: service.set_program_active(&tenant, &principal, id, input.active).await?.into(),
    })
}

/// Get the loyalty points of a customer.
///
/// Requires `customers.read` covering the customer's unit.
#[utoipa::path(
    get, path = "/{id}/points",
    extensions(("x-permissions" = json!(["customers.read"]))),
    operation_id = "get_customer_points", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer profile.")),
    responses(
        (status = 200, description = "Balance, lots and the rule in force.", body = Data<CustomerPoints>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No customer with this id in your scope; code `customers.profile_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_points(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CustomerPoints>, Error> {
    principal.require(&READ)?;
    let row = service.get(&tenant, &principal, id).await?;
    Ok(Data { data: CustomerPoints::new(service.points_view(&tenant, row.id).await?)? })
}

/// List the points history of a customer.
///
/// Requires `customers.read` covering the customer's unit. Newest first unless `order=asc`.
#[utoipa::path(
    get, path = "/{id}/points/entries",
    extensions(("x-permissions" = json!(["customers.read"]))),
    operation_id = "list_customer_points_entries", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer profile."), PageQuery),
    responses(
        (status = 200, description = "One page of the history.", body = Page<PointsEntry>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No customer with this id in your scope; code `customers.profile_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_points_entries(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<PointsEntry>, Error> {
    principal.require(&READ)?;
    let row = service.get(&tenant, &principal, id).await?;
    let page = service.points_entries(&tenant, row.id, &PageRequest::try_from(page)?).await?;
    Ok(page.map(PointsEntry::from))
}

/// Add or remove points of a customer by hand.
///
/// Requires `customers.loyalty.adjust` covering the customer's unit (and `customers.read`). The
/// same `reference` never applies twice; every change is in the history with its reason.
#[utoipa::path(
    post, path = "/{id}/points/adjustments",
    extensions(("x-permissions" = json!(["customers.loyalty.adjust"]))),
    operation_id = "adjust_customer_points", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer profile.")),
    request_body = AdjustPointsRequest,
    responses(
        (status = 200, description = "Points after the change.", body = Data<CustomerPoints>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `customers.loyalty.adjust`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No customer with this id in your scope; code `customers.profile_not_found`.", body = Problem),
        (status = 409, description = "Not enough points to remove (`customers.insufficient_points`) or no loyalty rule yet (`customers.no_loyalty_rule`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn adjust_points(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AdjustPointsRequest>,
) -> Result<Data<CustomerPoints>, Error> {
    principal.require(&LOYALTY_ADJUST)?;
    let view = service
        .adjust_points(&tenant, &principal, id, input.points, &input.reference, &input.reason)
        .await?;
    Ok(Data { data: CustomerPoints::new(view)? })
}

/// Get your loyalty points and the program rules.
///
/// Any signed-in buyer. Pay with points at checkout with the tender `points`.
#[utoipa::path(
    get, path = "/me/points",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_points", security(("bearer" = [])),
    responses(
        (status = 200, description = "Your balance, lots and the rule in force.", body = Data<CustomerPoints>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
pub(crate) async fn get_my_points(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<CustomerPoints>, Error> {
    let row = service.me(&tenant, principal.user_id()).await?;
    Ok(Data { data: CustomerPoints::new(service.points_view(&tenant, row.id).await?)? })
}

/// List the voucher rewards you can get for points.
///
/// Any signed-in buyer.
#[utoipa::path(
    get, path = "/me/points/rewards",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_point_rewards", security(("bearer" = [])), params(PageQuery),
    responses(
        (status = 200, description = "One page of active rewards.", body = Page<VoucherReward>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_my_rewards(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    _principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<VoucherReward>, Error> {
    let page = service.programs_page(&tenant, &PageRequest::try_from(page)?, true).await?;
    let mut page = page.map(VoucherReward::from);
    page.data.retain(|reward| reward.kind == crate::loyalty::vouchers::POINTS);
    Ok(page)
}

/// Exchange your points for a voucher.
///
/// Any signed-in buyer. The points are taken first; if the voucher cannot be granted they come
/// back and the error says why. Send an `Idempotency-Key` so a retry does not exchange twice.
#[utoipa::path(
    post, path = "/me/points/vouchers",
    extensions(("x-permissions" = json!([]))),
    operation_id = "redeem_my_points_for_voucher", security(("bearer" = [])), request_body = RedeemVoucherRequest,
    responses(
        (status = 201, description = "The voucher is in your wallet.", body = Data<RedeemedVoucher>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No active reward with this id; code `customers.voucher_program_not_found`.", body = Problem),
        (status = 409, description = "Not enough points (`customers.insufficient_points`) or the voucher ran out (promotions codes).", body = Problem),
        (status = 503, description = "The promotions module is not installed; code `customers.vouchers_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn redeem_voucher(
    State(service): State<Arc<CustomersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<RedeemVoucherRequest>,
) -> Result<Created<RedeemedVoucher>, Error> {
    let voucher = service.redeem_for_voucher(&tenant, principal.user_id(), input.reward_id).await?;
    Ok(Created::new(
        "/promotions/me/vouchers".to_owned(),
        RedeemedVoucher { voucher_id: voucher.id, promotion_id: voucher.promotion_id },
    ))
}
