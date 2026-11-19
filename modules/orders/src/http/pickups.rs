//! Endpoint nhận tại cửa hàng: báo sẵn sàng (gửi mã nhận), giao ở quầy.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::dto::OrderResponse;
use crate::pickups::Collector;
use crate::service::OrdersService;

/// An order ready in store.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersPickupReady)]
pub(crate) struct PickupReadyResponse {
    order_id: Uuid,
    pickup_org_id: Uuid,
    /// `ready`, `picked_up` or `expired`.
    status: String,
    ready_at: DateTime<Utc>,
    /// Held until then; afterwards the shop's `orders.pickup_expired_action` applies.
    hold_until: DateTime<Utc>,
}

/// Mark an order ready for collection.
///
/// Requires `orders.manage` at the order's shop. For an order collected in store that is
/// `confirmed` (accepted now) or `preparing`: the buyer gets a 6-digit pickup code and the items
/// are held for the pickup method's hold days. Sent again: the same pickup.
#[utoipa::path(
    post,
    path = "/{id}/pickup/ready",
    extensions(("x-permissions" = json!(["orders.manage"]))),
    operation_id = "mark_order_pickup_ready",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    responses(
        (status = 200, description = "The pickup.", body = Data<PickupReadyResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.manage` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "Not collected in store, or not in a status to get ready; code `orders.pickup_not_allowed`.", body = Problem),
    )
)]
pub(crate) async fn ready(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PickupReadyResponse>, Error> {
    let row = service.pickup_ready(&tenant, &principal, id).await?;
    Ok(Data {
        data: PickupReadyResponse {
            order_id: row.order_id,
            pickup_org_id: row.pickup_org_id,
            status: row.status,
            ready_at: row.ready_at,
            hold_until: row.hold_until,
        },
    })
}

/// Who collects the order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersPickupHandoverRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandoverRequest {
    /// The 6-digit code the buyer shows.
    code: Option<String>,
    /// Or: the name of the person collecting for the buyer, as given when ordering.
    proxy_name: Option<String>,
    /// And that person's phone number.
    proxy_phone: Option<String>,
}

/// Hand an order over at the counter.
///
/// Requires `orders.manage` at the order's shop. Give the buyer's `code`, or the `proxy_name`
/// and `proxy_phone` given when ordering; the order becomes `delivered`. Sent again after the
/// handover: the order.
#[utoipa::path(
    post,
    path = "/{id}/pickup/handover",
    extensions(("x-permissions" = json!(["orders.manage"]))),
    operation_id = "hand_over_order_pickup",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    request_body = HandoverRequest,
    responses(
        (status = 200, description = "The delivered order.", body = Data<OrderResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.manage` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "Not ready yet (`orders.pickup_not_ready`) or not collected in store (`orders.pickup_not_allowed`).", body = Problem),
        (status = 422, description = "Wrong code or person (`orders.pickup_code_invalid`), or neither a code nor a person given (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn handover(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<HandoverRequest>,
) -> Result<Data<OrderResponse>, Error> {
    let collector = match (body.code, body.proxy_name, body.proxy_phone) {
        (Some(code), None, None) => Collector::Code(code),
        (None, Some(name), Some(phone)) => Collector::Proxy { name, phone },
        _ => {
            return Err(Error::validation(vec![FieldError::new(
                "code",
                "required",
                "give either `code`, or both `proxy_name` and `proxy_phone`",
            )]));
        }
    };
    let row = service.pickup_handover(&tenant, &principal, id, collector).await?;
    let view = service.order_view(&tenant, row).await?;
    Ok(Data { data: OrderResponse::new(view)? })
}
