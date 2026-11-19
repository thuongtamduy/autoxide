//! Endpoint đặt hàng và đọc đơn: `POST /checkout` (người mua đăng nhập hay vãng lai,
//! `Idempotency-Key` bắt buộc), `GET /public/payments/{token}`, `/me/orders`, và danh sách đơn của
//! nhân viên.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::carts::CART_TOKEN;
use super::dto::{CheckoutResponse, OrderResponse, PaymentActionResponse, PublicPaymentResponse};
use crate::carts::Owner;
use crate::checkout::{CheckoutInput, Contact};
use crate::errors;
use crate::orders_read::OrderFilter;
use crate::service::OrdersService;

/// Contact for the order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersContactRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContactRequest {
    name: String,
    /// Mobile number; delivery and order lookup use it.
    phone: String,
    email: Option<String>,
}

/// Place the order of a cart.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersCheckoutRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckoutRequest {
    cart_id: Uuid,
    /// `quote_token` of the cart you showed the buyer.
    quote_token: String,
    /// `cod`, `bank_transfer` (VietQR to the seller's account) or a method of the payment
    /// partner (`card`, `wallet`…).
    #[validate(length(min = 1, max = 32))]
    payment_method: String,
    contact: ContactRequest,
    #[validate(length(max = 1000))]
    note: Option<String>,
    /// Version of the terms of sale the buyer accepted.
    #[validate(length(min = 1, max = 40))]
    accept_terms_version: Option<String>,
    /// Company details for a VAT invoice (tax code, name, address).
    #[schema(value_type = Option<Object>)]
    invoice_request: Option<serde_json::Value>,
    /// Where the payment partner sends the buyer back.
    #[validate(length(min = 1, max = 2000))]
    return_url: Option<String>,
}

/// Place an order from a cart.
///
/// Public: signed-in buyer, or guest with the `Cart-Token` header. The `Idempotency-Key` header is
/// required: sending the same key again returns the same order (200), even days later. Prices are
/// computed again; when they differ from `quote_token` nothing is ordered (`orders.quote_changed`).
/// Stock and promotion uses are held until the payment deadline; `payment_actions` say what the
/// buyer does next (go to the payment partner, scan a QR code, pay on delivery).
#[utoipa::path(
    post,
    path = "/checkout",
    extensions(("x-public" = json!(true))),
    operation_id = "orders_checkout",
    security((), ("bearer" = [])),
    params(
        ("Idempotency-Key" = String, Header, description = "Unique key of this checkout attempt."),
        ("Cart-Token" = Option<String>, Header, description = "Token of a guest cart."),
    ),
    request_body = CheckoutRequest,
    responses(
        (status = 201, description = "Ordered.", body = Data<CheckoutResponse>),
        (status = 200, description = "Same `Idempotency-Key` as an earlier checkout: its result.", body = Data<CheckoutResponse>),
        (status = 401, description = "Guest cart without the right `Cart-Token`; code `orders.cart_token_invalid`.", body = Problem),
        (status = 404, description = "No such cart; code `orders.cart_not_found`.", body = Problem),
        (status = 409, description = "Prices changed (`orders.quote_changed`) or the quote expired (`orders.quote_expired`), not enough stock (`orders.out_of_stock`), a promotion ran out (`orders.promotion_unavailable`), the cart was already ordered (`orders.cart_closed`), or the same key is still being processed (`orders.checkout_in_progress`) or failed (`orders.checkout_failed`).", body = Problem),
        (status = 422, description = "Empty cart (`orders.cart_empty`), delivery missing (`orders.delivery_required`), payment method not usable (`orders.payment_method_unavailable`), a product not on sale (`orders.variant_unavailable`), the key reused for another cart (`orders.idempotency_key_reused`), or invalid values (see `errors`).", body = Problem),
        (status = 428, description = "The `Idempotency-Key` header is missing; code `orders.idempotency_key_required`.", body = Problem),
    )
)]
pub(crate) async fn checkout(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    headers: HeaderMap,
    ValidJson(input): ValidJson<CheckoutRequest>,
) -> Result<Response, Error> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(errors::idempotency_key_required)?
        .to_owned();
    let token = headers.get(CART_TOKEN).and_then(|value| value.to_str().ok());
    let owner = Owner::of(principal.as_ref(), token).ok_or_else(errors::cart_token_invalid)?;
    let request = CheckoutInput {
        cart_id: input.cart_id,
        quote_token: input.quote_token,
        payment_method: input.payment_method,
        contact: Contact {
            name: input.contact.name,
            phone: input.contact.phone,
            email: input.contact.email,
        },
        note: input.note,
        accept_terms_version: input.accept_terms_version,
        invoice_request: input.invoice_request,
        return_url: input.return_url,
    };
    let placed = service.checkout(&tenant, &owner, &key, request).await?;
    let (created, placed) = match placed {
        Submitted::Created(placed) => (true, placed),
        Submitted::Replayed(placed) => (false, placed),
    };
    let mut orders = Vec::new();
    let mut actions = Vec::new();
    for id in &placed.order_ids {
        let view = service.order_view(&tenant, service.order(&tenant, *id).await?).await?;
        for payment in &view.payments {
            let status_token = service.status_token(&tenant, payment.id).await?;
            actions.push(PaymentActionResponse::new(payment, status_token)?);
        }
        orders.push(OrderResponse::new(view)?);
    }
    let body = CheckoutResponse { orders, payment_actions: actions };
    Ok(if created {
        let location = format!("/orders/me/orders/{}", placed.order_ids[0]);
        Created::new(location, body).into_response()
    } else {
        Data { data: body }.into_response()
    })
}

/// Status of a payment, for the QR code screen.
///
/// Public, with the `status_token` of a payment action; works until the payment deadline (and
/// after the payment succeeded). Poll it while the buyer pays.
#[utoipa::path(
    get,
    path = "/public/payments/{token}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_orders_public_payment",
    security(()),
    params(("token" = String, Path, description = "`status_token` of the payment action.")),
    responses(
        (status = 200, description = "The payment status.", body = Data<PublicPaymentResponse>),
        (status = 404, description = "Unknown or expired token; code `orders.payment_token_invalid`.", body = Problem),
    )
)]
pub(crate) async fn public_payment(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    Path(token): Path<String>,
) -> Result<Data<PublicPaymentResponse>, Error> {
    let (payment, order) = service.public_payment(&tenant, &token).await?;
    Ok(Data { data: PublicPaymentResponse::new(&payment, &order)? })
}

/// Your orders.
///
/// Signed in: orders you placed with your account, newest first by default.
#[utoipa::path(
    get,
    path = "/me/orders",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_orders",
    params(PageQuery),
    responses(
        (status = 200, description = "A page of your orders (lines omitted).", body = Page<OrderResponse>),
        (status = 400, description = "Bad cursor or limit.", body = Problem),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
    )
)]
pub(crate) async fn my_orders(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<OrderResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    let found = service.my_orders(&tenant, principal.user_id(), &page).await?;
    let pagination = found.pagination;
    let data = found.data.into_iter().map(OrderResponse::summary).collect::<Result<_, _>>()?;
    Ok(Page { data, pagination })
}

/// One of your orders.
///
/// Signed in: an order you placed with your account, with lines and payments.
#[utoipa::path(
    get,
    path = "/me/orders/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_order",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    responses(
        (status = 200, description = "The order.", body = Data<OrderResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such order of yours; code `orders.order_not_found`.", body = Problem),
    )
)]
pub(crate) async fn my_order(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<OrderResponse>, Error> {
    let view = service.my_order(&tenant, principal.user_id(), id).await?;
    Ok(Data { data: OrderResponse::new(view)? })
}

/// Filter of the staff order list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct OrderListQuery {
    /// `pending_payment`, `confirmed`, `preparing`, `shipping`, `delivered`, `delivery_failed`,
    /// `completed`, `cancelled`.
    status: Option<String>,
    /// `unpaid`, `cod_pending`, `paid`…
    payment_status: Option<String>,
    /// Only orders of this shop.
    org_id: Option<Uuid>,
    /// `web`, `pos` or a channel code.
    channel: Option<String>,
    /// The order number.
    number: Option<String>,
}

/// List orders.
///
/// Requires `orders.read` (orders of shops in your scope).
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-permissions" = json!(["orders.read"]))),
    operation_id = "list_orders",
    params(OrderListQuery, PageQuery),
    responses(
        (status = 200, description = "A page of orders (lines omitted), newest first by default.", body = Page<OrderResponse>),
        (status = 400, description = "Bad cursor or limit.", body = Problem),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.read`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn list_orders(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<OrderListQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<OrderResponse>, Error> {
    let filter = OrderFilter {
        status: query.status,
        payment_status: query.payment_status,
        org_id: query.org_id,
        channel: query.channel,
        number: query.number,
    };
    let page = PageRequest::try_from(page)?;
    let found = service.list_orders(&tenant, &principal, &filter, &page).await?;
    let pagination = found.pagination;
    let data = found.data.into_iter().map(OrderResponse::summary).collect::<Result<_, _>>()?;
    Ok(Page { data, pagination })
}

/// Read an order.
///
/// Requires `orders.read` at the order's shop.
#[utoipa::path(
    get,
    path = "/{id}",
    extensions(("x-permissions" = json!(["orders.read"]))),
    operation_id = "get_order",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    responses(
        (status = 200, description = "The order with lines and payments.", body = Data<OrderResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.read` at the order's shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order; code `orders.order_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_order(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<OrderResponse>, Error> {
    let view = service.staff_order(&tenant, &principal, id).await?;
    Ok(Data { data: OrderResponse::new(view)? })
}
