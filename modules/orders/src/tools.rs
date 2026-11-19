//! Tool MCP của module `orders` cho agent quản trị: cùng service, cùng permission với REST (luật
//! kiến trúc 8). Không có tool cho đặt hàng của người mua hay quầy (giao diện người dùng cuối).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::OrderStatus;
use crate::http::dto::OrderResponse;
use crate::orders_read::OrderFilter;
use crate::permissions::{MANAGE, READ};
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

pub(crate) fn all(service: &Arc<OrdersService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let s = || service.clone();
    let (list, get, transition, ship) = (s(), s(), s(), s());
    vec![
        Tool::new(
            "list_orders",
            "List orders of shops in your scope, newest first; follow `pagination.next_cursor` \
             for more.",
            move |tenant: TenantContext, principal: Principal, input: ListOrdersInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
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
    ]
}
