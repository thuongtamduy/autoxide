//! Tool MCP của module `orders` cho agent quản trị: cùng service, cùng permission với REST (luật
//! kiến trúc 8). Không có tool cho đặt hàng của người mua hay quầy (giao diện người dùng cuối).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::OrderStatus;
use crate::http::dto::OrderResponse;
use crate::http::returns::{
    ReceivedLineRequest, RefundResponse, ReturnResponse, ReturnTransitionRequest, return_action,
};
use crate::http::warranty::{ClaimResponse, WarrantyResponse};
use crate::orders_read::OrderFilter;
use crate::permissions::{MANAGE, READ, REFUND, RETURNS_MANAGE, WARRANTY_MANAGE};
use crate::returns::ReturnFilter;
use crate::service::OrdersService;

/// Orders to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = OrdersListOrdersInput)]
pub(crate) struct ListOrdersInput {
    /// Only orders in this status (`pending_payment`, `confirmed`, `preparing`, `shipping`,
    /// `delivered`, `delivery_failed`, `completed`, `cancelled`).
    pub status: Option<String>,
    /// Only orders of this shop.
    pub org_id: Option<Uuid>,
    /// The order number.
    pub number: Option<String>,
    /// Maximum number of orders, 1 to 200. Defaults to 50.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    pub count: Option<bool>,
}

/// One order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = OrdersOrderIdInput)]
pub(crate) struct OrderIdInput {
    pub id: Uuid,
}

/// A status change of an order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = OrdersTransitionOrderInput)]
pub(crate) struct TransitionOrderInput {
    pub id: Uuid,
    /// `preparing`, `shipping`, `delivered`, `delivery_failed` or `cancelled`.
    pub to: String,
    /// Why (kept in the history; shown to the buyer when cancelling).
    #[validate(length(min = 1, max = 200))]
    pub reason: Option<String>,
}

/// A shipment to request.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = OrdersRequestShipmentInput)]
pub(crate) struct RequestShipmentInput {
    pub id: Uuid,
    /// Note for the carrier.
    #[validate(length(min = 1, max = 500))]
    pub note: Option<String>,
}

/// A refund of an order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = OrdersRefundOrderInput)]
pub(crate) struct RefundOrderInput {
    pub id: Uuid,
    /// Integer amount in minor units, e.g. `"150000"` for 150,000 VND.
    pub amount: String,
    /// Currency of the order, e.g. `VND`.
    pub currency: String,
    /// Why (no personal data; sent to the payment partner).
    pub reason: String,
    /// Your unique key of this refund: sending it again returns the same refund.
    pub reference: String,
}

/// Returns to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = OrdersListReturnsInput)]
pub(crate) struct ListReturnsInput {
    /// `requested`, `approved`, `rejected`, `received`, `refunded` or `closed`.
    pub status: Option<String>,
    pub org_id: Option<Uuid>,
    pub order_id: Option<Uuid>,
    /// Maximum number of returns, 1 to 200. Defaults to 50.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    pub count: Option<bool>,
}

/// A step of a return.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = OrdersDecideReturnInput)]
pub(crate) struct DecideReturnInput {
    pub id: Uuid,
    /// `approve`, `reject`, `receive`, `refund` or `close`.
    pub action: String,
    /// `reject`: a code of the `orders.return_rejected` list.
    pub reason: Option<String>,
    /// `receive`: what arrived (`line_id`, `quantity`, `condition` = `restock` / `damaged`);
    /// lines left out count as received in full and sellable.
    #[schema(inline)]
    pub lines: Option<Vec<ReceivedLineRequest>>,
    /// `receive`: warehouse that received the items.
    pub warehouse_org_id: Option<Uuid>,
}

/// Warranties to find.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = OrdersLookupWarrantyInput)]
pub(crate) struct LookupWarrantyInput {
    /// Serial number printed on the product.
    pub serial: Option<String>,
    /// Phone number the buyer ordered with.
    pub phone: Option<String>,
    /// Maximum number of warranties, 1 to 200. Defaults to 50.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc` (oldest first).
    pub order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    pub count: Option<bool>,
}

/// A status change of a warranty claim.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = OrdersUpdateWarrantyClaimInput)]
pub(crate) struct UpdateWarrantyClaimInput {
    pub id: Uuid,
    /// `inspecting`, `repairing`, `replacing`, `ready`, `returned` or `rejected`.
    pub to: String,
    /// What was done (or why it was rejected).
    #[validate(length(min = 1, max = 5000, message = "must be 1 to 5000 characters"))]
    pub resolution: Option<String>,
}

pub(crate) fn all(service: &Arc<OrdersService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let s = || service.clone();
    let (list, get, transition, ship) = (s(), s(), s(), s());
    let (refund, returns, decide) = (s(), s(), s());
    let (lookup, claim) = (s(), s());
    vec![
        Tool::new(
            "list_orders",
            "List orders of shops in your scope, newest first; follow `pagination.next_cursor` \
             for more.",
            move |tenant: TenantContext, principal: Principal, input: ListOrdersInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let filter = OrderFilter {
                        status: input.status,
                        org_id: input.org_id,
                        number: input.number,
                        ..OrderFilter::default()
                    };
                    let found = service.list_orders(&tenant, &principal, &filter, &page).await?;
                    let pagination = found.pagination;
                    let data = found
                        .data
                        .into_iter()
                        .map(OrderResponse::summary)
                        .collect::<Result<_, _>>()?;
                    Ok::<Page<OrderResponse>, Error>(Page { data, pagination })
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "get_order",
            "Read an order with its lines, totals and payment status.",
            move |tenant: TenantContext, principal: Principal, input: OrderIdInput| {
                let service = get.clone();
                async move {
                    let view = service.staff_order(&tenant, &principal, input.id).await?;
                    OrderResponse::new(view)
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "transition_order",
            "Move an order through its lifecycle: accept it (`preparing`), mark it shipping, \
             delivered or failed, or cancel it (a paid order is refunded through the payment \
             partner).",
            move |tenant: TenantContext, principal: Principal, input: TransitionOrderInput| {
                let service = transition.clone();
                async move {
                    let to = OrderStatus::parse(&input.to).ok_or_else(|| {
                        Error::validation(vec![FieldError::new(
                            "to",
                            "invalid",
                            "is not an order status",
                        )])
                    })?;
                    let row = service
                        .staff_transition(&tenant, &principal, input.id, to, input.reason, None)
                        .await?;
                    OrderResponse::new(service.order_view(&tenant, row).await?)
                }
            },
        )
        .requires(MANAGE)
        .destructive(),
        Tool::new(
            "request_shipment",
            "Ask the shipping module for a shipment of an order's items (accepting the order \
             first when it is confirmed); cash-on-delivery orders carry the amount to collect.",
            move |tenant: TenantContext, principal: Principal, input: RequestShipmentInput| {
                let service = ship.clone();
                async move {
                    let request =
                        service.request_shipment(&tenant, &principal, input.id, input.note).await?;
                    Ok::<_, Error>(serde_json::json!({
                        "order_id": input.id, "request_key": request.request_key,
                        "cod_amount": request.cod_amount,
                    }))
                }
            },
        )
        .requires(MANAGE),
        Tool::new(
            "refund_order",
            "Refund part or all of an order: through the payment partner when the payment came \
             through it, otherwise recorded as paid back by the shop (cash). Never more than was \
             paid; the same `reference` returns the same refund.",
            move |tenant: TenantContext, principal: Principal, input: RefundOrderInput| {
                let service = refund.clone();
                async move {
                    let amount = Money::parse(&input.amount, &input.currency)?;
                    let row = service
                        .staff_refund(
                            &tenant,
                            &principal,
                            input.id,
                            amount,
                            input.reason,
                            input.reference,
                        )
                        .await?
                        .into_inner();
                    RefundResponse::new(row)
                }
            },
        )
        .requires(REFUND)
        .destructive()
        .requires_approval_for_agent(),
        Tool::new(
            "list_returns",
            "List return requests of shops in your scope, newest first, with lines, photos \
             (`media_ids`) and the refund once computed; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: ListReturnsInput| {
                let service = returns.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let filter = ReturnFilter {
                        status: input.status,
                        org_id: input.org_id,
                        order_id: input.order_id,
                    };
                    let found = service.list_returns(&tenant, &principal, &filter, &page).await?;
                    let pagination = found.pagination;
                    let data = found
                        .data
                        .into_iter()
                        .map(ReturnResponse::new)
                        .collect::<Result<_, _>>()?;
                    Ok::<Page<ReturnResponse>, Error>(Page { data, pagination })
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "decide_return",
            "Move a return forward: `approve` or `reject` (with a reason code of \
             `orders.return_rejected`) the request, `receive` the items, `refund` the computed \
             amount, or `close` it. Returns the return with its photos (`media_ids`).",
            move |tenant: TenantContext, principal: Principal, input: DecideReturnInput| {
                let service = decide.clone();
                async move {
                    let body = ReturnTransitionRequest {
                        action: input.action,
                        reason: input.reason,
                        lines: input.lines,
                        warehouse_org_id: input.warehouse_org_id,
                    };
                    let action =
                        return_action(&service, &tenant, &principal, input.id, body).await?;
                    let row =
                        service.transition_return(&tenant, &principal, input.id, action).await?;
                    ReturnResponse::new(row)
                }
            },
        )
        .requires(RETURNS_MANAGE)
        .destructive(),
        Tool::new(
            "lookup_warranty",
            "Find warranties of shops in your scope by `serial` or buyer `phone` (either matches): \
             product, order line, start, end and `active` / `expired`. Newest first; follow \
             `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: LookupWarrantyInput| {
                let service = lookup.clone();
                async move {
                    // Cùng kiểu tham số trang với REST (`PageQuery`, mặc định `desc`).
                    let page = PageInput::new(input.limit, input.cursor, input.order)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let rows = service
                        .find_warranties(
                            &tenant,
                            &principal,
                            input.serial.as_deref(),
                            input.phone.as_deref(),
                            &page,
                        )
                        .await?;
                    Ok::<Page<WarrantyResponse>, Error>(rows.map(WarrantyResponse::new))
                }
            },
        )
        // Không đánh dấu `read_only`: `orders.warranty.manage` là quyền mức ghi (tiếp nhận, xử lý
        // phiếu), không có quyền mức đọc riêng cho tra bảo hành.
        .requires(WARRANTY_MANAGE),
        Tool::new(
            "update_warranty_claim",
            "Move a warranty claim on: `received` → `inspecting` / `rejected`; `inspecting` → \
             `repairing` / `replacing` / `rejected`; `repairing` / `replacing` → `ready`; `ready` → \
             `returned`. Optionally record what was done in `resolution`.",
            move |tenant: TenantContext, principal: Principal, input: UpdateWarrantyClaimInput| {
                let service = claim.clone();
                async move {
                    let row = service
                        .transition_claim(&tenant, &principal, input.id, &input.to, input.resolution)
                        .await?;
                    Ok::<ClaimResponse, Error>(ClaimResponse::new(row))
                }
            },
        )
        .requires(WARRANTY_MANAGE),
    ]
}
