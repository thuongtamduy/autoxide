//! Endpoint chống bom hàng COD (D-194) và báo cáo bán hàng (D-213).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::entity::{cod_block, cod_rule};
use crate::reports::GroupBy;
use crate::risk::NewRule;
use crate::service::OrdersService;

/// Delivery statistics of a buyer, in this shop only.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersBuyerRisk)]
pub(crate) struct BuyerRiskResponse {
    delivered: i64,
    /// Delivery attempts that failed.
    failed: i64,
    /// Orders cancelled after they left (refused, returned).
    refused: i64,
    /// `(failed + refused) / all`, in basis points (10000 = 100%).
    fail_ratio_bp: i64,
    /// Until when cash on delivery is blocked, if it is.
    blocked_until: Option<Timestamp>,
    block_reason: Option<String>,
}

/// Look up a buyer by phone.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BuyerQuery {
    /// Phone number of the buyer.
    phone: String,
}

/// Delivery statistics of a buyer.
///
/// Requires `orders.risk.manage`. Only orders of this shop (tenant) count; nothing is shared
/// between tenants.
#[utoipa::path(
    get,
    path = "/risk/buyers",
    extensions(("x-permissions" = json!(["orders.risk.manage"]))),
    operation_id = "get_buyer_risk",
    params(BuyerQuery),
    responses(
        (status = 200, description = "The statistics.", body = Data<BuyerRiskResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.risk.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Not a phone number (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn buyer_risk(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<BuyerQuery>,
) -> Result<Data<BuyerRiskResponse>, Error> {
    let (stats, block) = service.risk_of_phone(&tenant, &principal, &query.phone).await?;
    Ok(Data {
        data: BuyerRiskResponse {
            delivered: stats.delivered,
            failed: stats.failed,
            refused: stats.refused,
            fail_ratio_bp: stats.fail_ratio_bp(),
            blocked_until: block.as_ref().map(|block| block.expires_at),
            block_reason: block.map(|block| block.reason_code),
        },
    })
}

/// A cash-on-delivery rule.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCodRule)]
pub(crate) struct RuleResponse {
    id: Uuid,
    org_id: Uuid,
    min_orders: i32,
    max_fail_ratio_bp: i32,
    max_order_value: Option<Money>,
    /// `require_prepayment`.
    action: String,
    effective_from: Timestamp,
    effective_to: Option<Timestamp>,
}

impl RuleResponse {
    fn new(row: cod_rule::Model) -> Result<Self, Error> {
        let max_order_value = match (row.max_order_value_minor, row.currency.as_deref()) {
            (Some(minor), Some(currency)) => Some(Money::from_columns(minor, currency)?),
            _ => None,
        };
        Ok(RuleResponse {
            id: row.id,
            org_id: row.org_id,
            min_orders: row.min_orders,
            max_fail_ratio_bp: row.max_fail_ratio_bp,
            max_order_value,
            action: row.action,
            effective_from: row.effective_from,
            effective_to: row.effective_to,
        })
    }
}

/// List cash-on-delivery rules.
///
/// Requires `orders.risk.manage` (rules of nodes in your scope), newest first.
#[utoipa::path(
    get,
    path = "/risk/cod-rules",
    extensions(("x-permissions" = json!(["orders.risk.manage"]))),
    operation_id = "list_cod_rules",
    responses(
        (status = 200, description = "The rules, ended ones included.", body = Data<Vec<RuleResponse>>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.risk.manage`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn list_rules(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<RuleResponse>>, Error> {
    let rows = service.cod_rules(&tenant, &principal).await?;
    Ok(Data { data: rows.into_iter().map(RuleResponse::new).collect::<Result<_, _>>()? })
}

/// A new cash-on-delivery rule.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersCodRuleRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleRequest {
    /// The shop (or the tenant's root node for every shop).
    org_id: Uuid,
    /// Orders with a delivery result the buyer needs before the ratio counts (1 to 1000).
    min_orders: i32,
    /// Failed or refused deliveries, in basis points, from which the buyer must pay in advance.
    max_fail_ratio_bp: i32,
    /// Orders above this total must be paid in advance, whoever the buyer.
    max_order_value: Option<Money>,
    /// Default: now.
    effective_from: Option<Timestamp>,
    effective_to: Option<Timestamp>,
}

/// Add a cash-on-delivery rule.
///
/// Requires `orders.risk.manage` at the node. A buyer whose failed and refused deliveries reach the
/// ratio (after `min_orders`), or an order above `max_order_value`, must pay in advance. Rules are
/// never edited: end one (`DELETE`) and add another.
#[utoipa::path(
    post,
    path = "/risk/cod-rules",
    extensions(("x-permissions" = json!(["orders.risk.manage"]))),
    operation_id = "create_cod_rule",
    request_body = RuleRequest,
    responses(
        (status = 201, description = "The rule.", body = Data<RuleResponse>, headers(("Location" = String, description = "Path of the rule."))),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.risk.manage` at the node; code `permission_denied`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn create_rule(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<RuleRequest>,
) -> Result<Created<RuleResponse>, Error> {
    let rule = NewRule {
        org_id: OrgId::from_uuid(input.org_id),
        min_orders: input.min_orders,
        max_fail_ratio_bp: input.max_fail_ratio_bp,
        max_order_value: input.max_order_value,
        effective_from: input.effective_from,
        effective_to: input.effective_to,
    };
    let row = service.add_cod_rule(&tenant, &principal, rule).await?;
    let path = format!("/orders/risk/cod-rules/{}", row.id);
    Ok(Created::new(path, RuleResponse::new(row)?))
}

/// End a cash-on-delivery rule.
///
/// Requires `orders.risk.manage` at the node. The rule stops applying now; it is kept.
#[utoipa::path(
    delete,
    path = "/risk/cod-rules/{id}",
    extensions(("x-permissions" = json!(["orders.risk.manage"]))),
    operation_id = "end_cod_rule",
    params(("id" = Uuid, Path, description = "Id of the rule.")),
    responses(
        (status = 200, description = "The ended rule.", body = Data<RuleResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.risk.manage` at the node; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such rule; code `orders.cod_rule_not_found`.", body = Problem),
    )
)]
pub(crate) async fn end_rule(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<RuleResponse>, Error> {
    let row = service.end_cod_rule(&tenant, &principal, id).await?;
    Ok(Data { data: RuleResponse::new(row)? })
}

/// A buyer blocked from paying on delivery.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCodBlock)]
pub(crate) struct BlockResponse {
    id: Uuid,
    /// Code of the list `orders.cod_block`.
    reason_code: String,
    note: Option<String>,
    expires_at: Timestamp,
    created_by: Uuid,
    created_at: Timestamp,
    lifted_at: Option<Timestamp>,
}

impl From<cod_block::Model> for BlockResponse {
    fn from(row: cod_block::Model) -> Self {
        BlockResponse {
            id: row.id,
            reason_code: row.reason_code,
            note: row.note,
            expires_at: row.expires_at,
            created_by: row.created_by,
            created_at: row.created_at,
            lifted_at: row.lifted_at,
        }
    }
}

/// List active cash-on-delivery blocks.
///
/// Requires `orders.risk.manage`. Blocks of this tenant only, expiring soonest first.
#[utoipa::path(
    get,
    path = "/risk/cod-blocks",
    extensions(("x-permissions" = json!(["orders.risk.manage"]))),
    operation_id = "list_cod_blocks",
    responses(
        (status = 200, description = "The active blocks.", body = Data<Vec<BlockResponse>>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.risk.manage`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn list_blocks(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<BlockResponse>>, Error> {
    let rows = service.cod_blocks(&tenant, &principal).await?;
    Ok(Data { data: rows.into_iter().map(BlockResponse::from).collect() })
}

/// Block a buyer from paying on delivery.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersCodBlockRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct BlockRequest {
    /// Phone number of the buyer (kept only as a keyed hash).
    #[validate(length(min = 8, max = 20))]
    #[schema(example = "0901234567")]
    phone: String,
    /// A code of the list `orders.cod_block`.
    #[schema(example = "fake_orders")]
    #[validate(length(min = 1, max = 64))]
    reason_code: String,
    #[validate(length(min = 1, max = 1000))]
    note: Option<String>,
    /// Required; at most 365 days from now (no block is permanent).
    expires_at: Timestamp,
}

/// Block a buyer from paying on delivery.
///
/// Requires `orders.risk.manage`. Needs a listed reason and an end date; the buyer must pay in
/// advance until then. Applies in this tenant only.
#[utoipa::path(
    post,
    path = "/risk/cod-blocks",
    extensions(("x-permissions" = json!(["orders.risk.manage"]))),
    operation_id = "create_cod_block",
    request_body = BlockRequest,
    responses(
        (status = 201, description = "The block.", body = Data<BlockResponse>, headers(("Location" = String, description = "Path of the block."))),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.risk.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn create_block(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<BlockRequest>,
) -> Result<Created<BlockResponse>, Error> {
    let row = service
        .block_cod(
            &tenant,
            &principal,
            &input.phone,
            &input.reason_code,
            input.note,
            input.expires_at,
        )
        .await?;
    let path = format!("/orders/risk/cod-blocks/{}", row.id);
    Ok(Created::new(path, row.into()))
}

/// Lift a cash-on-delivery block.
///
/// Requires `orders.risk.manage`. Lifting again changes nothing.
#[utoipa::path(
    delete,
    path = "/risk/cod-blocks/{id}",
    extensions(("x-permissions" = json!(["orders.risk.manage"]))),
    operation_id = "lift_cod_block",
    params(("id" = Uuid, Path, description = "Id of the block.")),
    responses(
        (status = 200, description = "The lifted block.", body = Data<BlockResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.risk.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such block; code `orders.cod_block_not_found`.", body = Problem),
    )
)]
pub(crate) async fn lift_block(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BlockResponse>, Error> {
    let row = service.lift_cod_block(&tenant, &principal, id).await?;
    Ok(Data { data: row.into() })
}

/// Sales report filters.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SalesQuery {
    /// `org`, `channel`, `product`, `day` or `month` (days and months in UTC).
    group_by: String,
    /// From (included).
    from: Timestamp,
    /// To (excluded); at most 366 days after `from`.
    to: Timestamp,
    /// Currency of the orders counted; default `VND`.
    currency: Option<String>,
}

/// A row of the sales report.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersSalesRow)]
pub(crate) struct SalesRowResponse {
    /// Shop id, channel code, product id, `YYYY-MM-DD` or `YYYY-MM`.
    key: String,
    orders: i64,
    /// Units sold (by product only).
    quantity: Option<i64>,
    /// What buyers paid: after discounts, taxes included.
    gross: Money,
    discount: Money,
    tax: Money,
    /// Refunded so far (not by product).
    refunded: Money,
}

/// Sales report.
///
/// Requires `orders.reports.read`; only shops in your scope count. Orders placed in `[from, to)`,
/// neither waiting for payment nor cancelled.
#[utoipa::path(
    get,
    path = "/reports/sales",
    extensions(("x-permissions" = json!(["orders.reports.read"]))),
    operation_id = "sales_report",
    params(SalesQuery),
    responses(
        (status = 200, description = "The rows (biggest first; days and months in order).", body = Data<Vec<SalesRowResponse>>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.reports.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn sales_report(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<SalesQuery>,
) -> Result<Data<Vec<SalesRowResponse>>, Error> {
    // Không có quyền thì 403 trước khi xét tham số.
    principal.require(&crate::permissions::REPORTS_READ)?;
    let mut problems = Vec::new();
    let group_by = GroupBy::parse(&query.group_by);
    if group_by.is_none() {
        problems.push(FieldError::new(
            "group_by",
            "invalid",
            "must be `org`, `channel`, `product`, `day` or `month`",
        ));
    }
    let currency = Currency::parse(query.currency.as_deref().unwrap_or("VND")).ok();
    if currency.is_none() {
        problems.push(FieldError::new("currency", "invalid", "is not a currency code"));
    }
    let (Some(group_by), Some(currency)) = (group_by, currency) else {
        return Err(Error::validation(problems));
    };
    let rows =
        service.sales_report(&tenant, &principal, group_by, query.from, query.to, currency).await?;
    let money = |minor: i64| Money::new(minor, currency);
    Ok(Data {
        data: rows
            .into_iter()
            .map(|row| SalesRowResponse {
                key: row.key,
                orders: row.orders,
                quantity: row.quantity,
                gross: money(row.gross),
                discount: money(row.discount),
                tax: money(row.tax),
                refunded: money(row.refunded),
            })
            .collect(),
    })
}
