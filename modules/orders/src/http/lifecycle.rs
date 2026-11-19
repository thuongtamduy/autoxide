//! Endpoint vòng đời đơn: nhân viên chuyển trạng thái, yêu cầu vận đơn, gửi lại link thanh toán;
//! người mua huỷ đơn của mình.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::dto::{OrderResponse, PaymentActionResponse};
use crate::api::OrderStatus;
use crate::service::OrdersService;

/// A status change of an order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersTransitionRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransitionRequest {
    /// `preparing` (accept the order), `shipping`, `delivered`, `delivery_failed` or `cancelled`.
    to: String,
    /// Why (kept in the order history; shown to the buyer when cancelling).
    #[validate(length(min = 1, max = 200))]
    reason: Option<String>,
}

/// Move an order through its lifecycle.
///
/// Requires `orders.manage` at the order's shop and the `If-Match` header with the order's
/// `ETag`. Only moves of the lifecycle table are allowed; cancelling a paid order refunds it
/// through the payment partner.
#[utoipa::path(
    post,
    path = "/{id}/transitions",
    extensions(("x-permissions" = json!(["orders.manage"]))),
    operation_id = "transition_order",
    params(("id" = Uuid, Path, description = "Id of the order."), ("If-Match" = String, Header, description = "The `ETag` you read.")),
    request_body = TransitionRequest,
    responses(
        (status = 200, description = "The order after the move.", body = Data<OrderResponse>, headers(("ETag" = String, description = "Version of the order."))),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.manage` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "The move is not in the lifecycle table; code `orders.invalid_transition`.", body = Problem),
        (status = 412, description = "The order changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "Unknown status (see `errors`).", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn transition(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<TransitionRequest>,
) -> Result<Tagged<OrderResponse>, Error> {
    let expected = if_match.required()?;
    let to = OrderStatus::parse(&input.to).ok_or_else(|| {
        Error::validation(vec![FieldError::new("to", "invalid", "is not an order status")])
    })?;
    let row = service.staff_transition(&tenant, &principal, id, to, input.reason, expected).await?;
    let version = row.updated_at;
    let view = service.order_view(&tenant, row).await?;
    Ok(Tagged { version, data: OrderResponse::new(view)? })
}

/// Cancel one of your orders.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = CancelMyOrderRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelRequest {
    #[validate(length(min = 1, max = 200))]
    reason: Option<String>,
}

/// Cancel one of your orders.
///
/// Signed in: an order you placed. Allowed until the status the shop's policy says
/// (`orders.buyer_cancel_until`, by default while the order is not being prepared yet); a paid
/// order is refunded through the payment partner.
#[utoipa::path(
    post,
    path = "/me/orders/{id}/cancel",
    extensions(("x-permissions" = json!([]))),
    operation_id = "cancel_my_order",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    request_body = CancelRequest,
    responses(
        (status = 200, description = "The cancelled order.", body = Data<OrderResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such order of yours; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "Too late to cancel by yourself; code `orders.cancel_not_allowed`.", body = Problem),
    )
)]
pub(crate) async fn cancel_mine(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CancelRequest>,
) -> Result<Data<OrderResponse>, Error> {
    let row = service.buyer_cancel(&tenant, principal.user_id(), id, input.reason).await?;
    let view = service.order_view(&tenant, row).await?;
    Ok(Data { data: OrderResponse::new(view)? })
}

/// A shipment to request.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = RequestOrderShipmentRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShipmentRequestBody {
    /// Note for the carrier.
    #[validate(length(min = 1, max = 500))]
    note: Option<String>,
}

/// A shipment requested from the shipping module.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersShipmentRequested)]
pub(crate) struct ShipmentRequestedResponse {
    order_id: Uuid,
    /// Key of the request at the shipping module.
    request_key: String,
    /// Cash to collect on delivery, if any.
    cod_amount: Option<autoxide_kernel::money::Money>,
}

/// Request a shipment for an order.
///
/// Requires `orders.manage` at the order's shop. Accepts the order (`preparing`) when it is
/// `confirmed`, then asks the shipping module for a shipment of the items to deliver (with the
/// cash to collect for cash-on-delivery orders). Sent again while preparing: the same request.
#[utoipa::path(
    post,
    path = "/{id}/shipments",
    extensions(("x-permissions" = json!(["orders.manage"]))),
    operation_id = "request_order_shipment",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    request_body = ShipmentRequestBody,
    responses(
        (status = 202, description = "Requested; the shipment appears at the shipping module.", body = Data<ShipmentRequestedResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.manage` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "The order cannot ship in its status; code `orders.shipment_not_allowed`.", body = Problem),
        (status = 422, description = "The order has nothing to deliver or no delivery address; code `orders.delivery_required`.", body = Problem),
    )
)]
pub(crate) async fn request_shipment(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ShipmentRequestBody>,
) -> Result<(axum::http::StatusCode, Data<ShipmentRequestedResponse>), Error> {
    let request = service.request_shipment(&tenant, &principal, id, input.note).await?;
    let data = ShipmentRequestedResponse {
        order_id: id,
        request_key: request.request_key,
        cod_amount: request.cod_amount,
    };
    Ok((axum::http::StatusCode::ACCEPTED, Data { data }))
}

/// Resend the payment of an order.
///
/// Requires `orders.manage` at the order's shop. For an order waiting for payment before its
/// deadline: the payment actions still valid (starting the payment again when it could not be
/// started at checkout), to send to the buyer.
#[utoipa::path(
    post,
    path = "/{id}/payment-link",
    extensions(("x-permissions" = json!(["orders.manage"]))),
    operation_id = "resend_order_payment_link",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    responses(
        (status = 200, description = "The payment actions of the order.", body = Data<Vec<PaymentActionResponse>>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.manage` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "The order is not waiting for payment (`orders.invalid_transition`) or its deadline passed (`orders.payment_expired`).", body = Problem),
    )
)]
pub(crate) async fn payment_link(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<PaymentActionResponse>>, Error> {
    let mut actions = Vec::new();
    for payment in service.payment_link(&tenant, &principal, id).await? {
        let token = service.status_token(&tenant, payment.id).await?;
        actions.push(PaymentActionResponse::new(&payment, token)?);
    }
    Ok(Data { data: actions })
}
