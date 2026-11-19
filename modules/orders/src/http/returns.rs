//! Endpoint đổi trả và hoàn tiền: người mua yêu cầu trả; nhân viên duyệt / nhận hàng / hoàn / đóng;
//! nhân viên hoàn tay và xem các lượt hoàn của đơn.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::entity::{refund, return_request};
use crate::return_refund::Breakdown;
use crate::returns::{self, Received, ReturnAction, ReturnFilter, ReturnInput};
use crate::service::OrdersService;

/// One line to return.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = OrdersReturnLineRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReturnLineRequest {
    /// Id of the order line.
    line_id: Uuid,
    /// Units to return (at most what is left of the line).
    quantity: i64,
}

/// Ask to return items of a delivered order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersReturnRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReturnRequestBody {
    lines: Vec<ReturnLineRequest>,
    /// A code of the shop's allowed return reasons (list `orders.return_reason`, see
    /// `GET /api/v1/kernel/reasons/orders.return_reason`).
    reason_code: String,
    #[validate(length(min = 1, max = 1000))]
    note: Option<String>,
    /// `refund` (default) or `exchange`.
    resolution: Option<String>,
    /// How the items go back: `pickup` (a carrier picks them up), `drop_off` (default: at a
    /// post office) or `in_store`.
    return_method: Option<String>,
    /// Photos uploaded by you (at most 6).
    #[serde(default)]
    media_ids: Vec<Uuid>,
}

/// One line of a return.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersReturnLine)]
pub(crate) struct ReturnLineResponse {
    line_id: Uuid,
    variant_id: Uuid,
    quantity: i64,
    /// A free gift sent back because the promotion's condition is no longer met.
    gift: bool,
    /// Units received back, once received.
    received: Option<i64>,
    /// `restock` or `damaged`, once received.
    condition: Option<String>,
}

/// How the refund of a return was computed.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersReturnRefundBreakdown)]
pub(crate) struct BreakdownResponse {
    /// What was paid for the returned units (after discounts).
    goods: Money,
    /// Share of the shipping fee (only when the seller is at fault).
    shipping: Money,
    /// Order discount taken back because the rest of the order is below the promotion minimum
    /// (negative: given back from an earlier return).
    clawback: Money,
    /// Value of free gifts kept although their condition is no longer met.
    gift_deduction: Money,
    total: Money,
}

/// A return.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersReturn)]
pub(crate) struct ReturnResponse {
    id: Uuid,
    order_id: Uuid,
    /// The shop of the order.
    org_id: Uuid,
    number: String,
    /// `requested`, `approved`, `rejected`, `received`, `refunded` or `closed`.
    status: String,
    reason_code: String,
    note: Option<String>,
    /// `refund` or `exchange`.
    resolution: String,
    /// `pickup`, `drop_off` or `in_store`.
    return_method: String,
    /// Photos sent with the request.
    media_ids: Vec<Uuid>,
    lines: Vec<ReturnLineResponse>,
    /// Amount to refund, computed when the items are received.
    refund: Option<Money>,
    refund_breakdown: Option<BreakdownResponse>,
    /// Code of the `orders.return_rejected` list when rejected.
    reject_reason: Option<String>,
    /// Warehouse that received the items.
    warehouse_org_id: Option<Uuid>,
    requested_at: DateTime<Utc>,
    /// End of the return window of the order.
    deadline_at: DateTime<Utc>,
    decided_at: Option<DateTime<Utc>>,
    received_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl ReturnResponse {
    pub(crate) fn new(row: return_request::Model) -> Result<Self, Error> {
        let money = |minor: i64| Money::from_columns(minor, &row.currency);
        let lines = returns::lines_of(&row)?
            .into_iter()
            .map(|line| ReturnLineResponse {
                line_id: line.line_id,
                variant_id: line.variant_id,
                quantity: line.quantity,
                gift: line.gift,
                received: line.received,
                condition: line.condition,
            })
            .collect();
        let breakdown: Option<Breakdown> =
            row.breakdown.as_deref().and_then(|raw| serde_json::from_str(raw).ok());
        let refund_breakdown = match breakdown {
            Some(found) => Some(BreakdownResponse {
                goods: money(found.goods)?,
                shipping: money(found.shipping)?,
                clawback: money(found.clawback)?,
                gift_deduction: money(found.gift_deduction)?,
                total: money(found.total)?,
            }),
            None => None,
        };
        Ok(ReturnResponse {
            media_ids: returns::media_of(&row),
            refund: row.refund_minor.map(money).transpose()?,
            refund_breakdown,
            lines,
            id: row.id,
            order_id: row.order_id,
            org_id: row.org_id,
            number: row.number,
            status: row.status,
            reason_code: row.reason_code,
            note: row.buyer_note,
            resolution: row.resolution,
            return_method: row.return_method,
            reject_reason: row.reject_reason,
            warehouse_org_id: row.warehouse_org_id,
            requested_at: row.requested_at,
            deadline_at: row.deadline_at,
            decided_at: row.decided_at,
            received_at: row.received_at,
            updated_at: row.updated_at,
        })
    }
}

/// Ask to return items.
///
/// Signed in: an order you placed, delivered and inside the shop's return window. Units already
/// returned or in another open return cannot be asked again. Free gifts are added when returning
/// makes the promotion's condition fail and the promotion wants gifts back. Send an
/// `Idempotency-Key` header to retry safely.
#[utoipa::path(
    post,
    path = "/me/orders/{id}/returns",
    extensions(("x-permissions" = json!([]))),
    operation_id = "request_my_return",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    request_body = ReturnRequestBody,
    responses(
        (status = 201, description = "The return, waiting for the shop.", body = Data<ReturnResponse>, headers(("Location" = String, description = "Where to read the return."))),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such order of yours; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "The order is not delivered (`orders.return_not_allowed`) or the return window ended (`orders.return_window_closed`).", body = Problem),
        (status = 422, description = "Invalid lines, quantities, reason, method or photos (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn request_return(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<ReturnRequestBody>,
) -> Result<Created<ReturnResponse>, Error> {
    let input = ReturnInput {
        lines: body.lines.iter().map(|line| (line.line_id, line.quantity)).collect(),
        reason_code: body.reason_code,
        note: body.note,
        resolution: body.resolution.unwrap_or_else(|| "refund".to_owned()),
        return_method: body.return_method.unwrap_or_else(|| "drop_off".to_owned()),
        media_ids: body.media_ids,
    };
    let row = service.request_return(&tenant, principal.user_id(), id, input).await?;
    let location = format!("/orders/me/returns/{}", row.id);
    Ok(Created::new(location, ReturnResponse::new(row)?))
}

/// One of your returns.
#[utoipa::path(
    get,
    path = "/me/returns/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_return",
    params(("id" = Uuid, Path, description = "Id of the return.")),
    responses(
        (status = 200, description = "The return.", body = Data<ReturnResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such return of yours; code `orders.return_not_found`.", body = Problem),
    )
)]
pub(crate) async fn my_return(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ReturnResponse>, Error> {
    let row = service.my_return(&tenant, principal.user_id(), id).await?;
    Ok(Data { data: ReturnResponse::new(row)? })
}

/// Filters of the return list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ReturnListQuery {
    /// `requested`, `approved`, `rejected`, `received`, `refunded` or `closed`.
    status: Option<String>,
    /// Only returns of orders of this shop.
    org_id: Option<Uuid>,
    /// Only returns of this order.
    order_id: Option<Uuid>,
}

/// List returns.
///
/// Requires `orders.read` (returns of orders of shops in your scope).
#[utoipa::path(
    get,
    path = "/returns",
    extensions(("x-permissions" = json!(["orders.read"]))),
    operation_id = "list_returns",
    params(ReturnListQuery, PageQuery),
    responses(
        (status = 200, description = "A page of returns, newest first by default.", body = Page<ReturnResponse>),
        (status = 400, description = "Bad cursor or limit.", body = Problem),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.read`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn list_returns(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ReturnListQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReturnResponse>, Error> {
    let filter =
        ReturnFilter { status: query.status, org_id: query.org_id, order_id: query.order_id };
    let page = PageRequest::try_from(page)?;
    let found = service.list_returns(&tenant, &principal, &filter, &page).await?;
    let pagination = found.pagination;
    let data = found.data.into_iter().map(ReturnResponse::new).collect::<Result<_, _>>()?;
    Ok(Page { data, pagination })
}

/// Read a return.
///
/// Requires `orders.read` at the order's shop. The photos are in `media_ids`.
#[utoipa::path(
    get,
    path = "/returns/{id}",
    extensions(("x-permissions" = json!(["orders.read"]))),
    operation_id = "get_return",
    params(("id" = Uuid, Path, description = "Id of the return.")),
    responses(
        (status = 200, description = "The return.", body = Data<ReturnResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.read` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such return; code `orders.return_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_return(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ReturnResponse>, Error> {
    let row = service.staff_return(&tenant, &principal, id, &crate::permissions::READ).await?;
    Ok(Data { data: ReturnResponse::new(row)? })
}

/// A line received back.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = OrdersReceivedLineRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReceivedLineRequest {
    pub(crate) line_id: Uuid,
    /// Units received (0 when missing).
    pub(crate) quantity: i64,
    /// `restock` (sellable again) or `damaged`.
    pub(crate) condition: String,
}

/// A step of a return.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersReturnTransitionRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReturnTransitionRequest {
    /// `approve`, `reject`, `receive`, `refund` or `close`.
    pub(crate) action: String,
    /// `reject`: a code of the `orders.return_rejected` list.
    pub(crate) reason: Option<String>,
    /// `receive`: what arrived; lines left out count as received in full and sellable.
    pub(crate) lines: Option<Vec<ReceivedLineRequest>>,
    /// `receive`: warehouse that received the items (default: the warehouse that shipped them).
    pub(crate) warehouse_org_id: Option<Uuid>,
}

/// Move a return forward.
///
/// Requires `orders.returns.manage` at the order's shop. `approve` and `reject` a request;
/// `receive` the items (restocked or damaged; the refund is computed then); `refund` pays the
/// computed amount back through the payment partner (or records a refund paid by the shop for
/// cash payments); `close` ends it (exchanges close after receiving).
#[utoipa::path(
    post,
    path = "/returns/{id}/transitions",
    extensions(("x-permissions" = json!(["orders.returns.manage"]))),
    operation_id = "transition_return",
    params(("id" = Uuid, Path, description = "Id of the return.")),
    request_body = ReturnTransitionRequest,
    responses(
        (status = 200, description = "The return after the step.", body = Data<ReturnResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.returns.manage` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such return; code `orders.return_not_found`.", body = Problem),
        (status = 409, description = "The step is not allowed now (`orders.return_invalid_transition`), or the refund is more than what is left to refund (`orders.refund_exceeds_paid`).", body = Problem),
        (status = 422, description = "Unknown action, reason, line or condition (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn transition_return(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<ReturnTransitionRequest>,
) -> Result<Data<ReturnResponse>, Error> {
    principal.require(&crate::permissions::RETURNS_MANAGE)?;
    let action = return_action(&service, &tenant, &principal, id, body).await?;
    let row = service.transition_return(&tenant, &principal, id, action).await?;
    Ok(Data { data: ReturnResponse::new(row)? })
}

/// Dựng `ReturnAction` từ thân request (REST và MCP dùng chung).
pub(crate) async fn return_action(
    service: &OrdersService,
    tenant: &TenantContext,
    principal: &Principal,
    id: Uuid,
    body: ReturnTransitionRequest,
) -> Result<ReturnAction, Error> {
    use autoxide_kernel::error::FieldError;
    Ok(match body.action.as_str() {
        "approve" => ReturnAction::Approve,
        "reject" => ReturnAction::Reject {
            reason: body.reason.ok_or_else(|| {
                Error::validation(vec![FieldError::new(
                    "reason",
                    "required",
                    "is required to reject",
                )])
            })?,
        },
        "receive" => {
            let mut lines: Vec<Received> = body
                .lines
                .unwrap_or_default()
                .into_iter()
                .map(|line| Received {
                    line_id: line.line_id,
                    quantity: line.quantity,
                    condition: line.condition,
                })
                .collect();
            // Dòng không nêu: nhận đủ, bán lại được.
            let stored = service
                .staff_return(tenant, principal, id, &crate::permissions::RETURNS_MANAGE)
                .await?;
            for line in returns::lines_of(&stored)? {
                if !lines.iter().any(|item| item.line_id == line.line_id) {
                    lines.push(Received {
                        line_id: line.line_id,
                        quantity: line.quantity,
                        condition: "restock".to_owned(),
                    });
                }
            }
            ReturnAction::Receive { lines, warehouse_org_id: body.warehouse_org_id }
        }
        "refund" => ReturnAction::Refund,
        "close" => ReturnAction::Close,
        _ => {
            return Err(Error::validation(vec![FieldError::new(
                "action",
                "invalid",
                "must be `approve`, `reject`, `receive`, `refund` or `close`",
            )]));
        }
    })
}

/// A refund of an order.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersRefund)]
pub(crate) struct RefundResponse {
    id: Uuid,
    order_id: Uuid,
    /// The return it pays back, if any.
    return_id: Option<Uuid>,
    /// The payment of the order it comes from.
    payment_id: Uuid,
    amount: Money,
    /// `pending` (the payment partner is refunding), `succeeded`, `failed`, or `manual` (paid
    /// back by the shop outside the payment partner, e.g. cash).
    status: String,
    reason: String,
    reference: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl RefundResponse {
    pub(crate) fn new(row: refund::Model) -> Result<Self, Error> {
        Ok(RefundResponse {
            amount: Money::from_columns(row.amount_minor, &row.currency)?,
            id: row.id,
            order_id: row.order_id,
            return_id: row.return_id,
            payment_id: row.payment_id,
            status: row.status,
            reason: row.reason,
            reference: row.reference,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// Refund part or all of an order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersRefundRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefundRequestBody {
    /// In the currency of the order; at most what is left to refund of one payment.
    amount: Money,
    /// Why (no personal data; sent to the payment partner).
    reason: String,
    /// Your unique key of this refund: sending it again returns the same refund.
    reference: String,
}

/// Refund an order.
///
/// Requires `orders.refund` at the order's shop. Paid through the payment partner when the
/// payment came through it; otherwise (cash on delivery, cash) recorded as `manual`: the shop
/// pays the buyer back itself. The total refunded never exceeds what was paid.
#[utoipa::path(
    post,
    path = "/{id}/refunds",
    extensions(("x-permissions" = json!(["orders.refund"]))),
    operation_id = "refund_order",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    request_body = RefundRequestBody,
    responses(
        (status = 201, description = "The refund.", body = Data<RefundResponse>, headers(("Location" = String, description = "The refunds of the order."))),
        (status = 200, description = "Same `reference` and content as an earlier refund: that refund.", body = Data<RefundResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.refund` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "More than what is left to refund; code `orders.refund_exceeds_paid`.", body = Problem),
        (status = 422, description = "Invalid amount, reason or reference, or the reference was used for another refund (`reference_reused`).", body = Problem),
    )
)]
pub(crate) async fn refund_order(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<RefundRequestBody>,
) -> Result<Response, Error> {
    let submitted = service
        .staff_refund(&tenant, &principal, id, body.amount, body.reason, body.reference)
        .await?;
    Ok(match submitted {
        Submitted::Created(row) => {
            Created::new(format!("/orders/{id}/refunds"), RefundResponse::new(row)?).into_response()
        }
        Submitted::Replayed(row) => Data { data: RefundResponse::new(row)? }.into_response(),
    })
}

/// Refunds of an order.
///
/// Requires `orders.read` at the order's shop. Oldest first.
#[utoipa::path(
    get,
    path = "/{id}/refunds",
    extensions(("x-permissions" = json!(["orders.read"]))),
    operation_id = "list_order_refunds",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    responses(
        (status = 200, description = "The refunds of the order.", body = Data<Vec<RefundResponse>>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.read` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order; code `orders.order_not_found`.", body = Problem),
    )
)]
pub(crate) async fn order_refunds(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<RefundResponse>>, Error> {
    let rows = service.order_refunds(&tenant, &principal, id).await?;
    let data = rows.into_iter().map(RefundResponse::new).collect::<Result<_, _>>()?;
    Ok(Data { data })
}
