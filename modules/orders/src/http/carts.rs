//! Endpoint giỏ hàng: người mua đăng nhập (bearer) hay vãng lai (header `Cart-Token`). Handler
//! chỉ: lấy dữ liệu vào → gọi service → trả envelope.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_promotions::api::Eligibility;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::dto::{CartResponse, CartSummaryResponse};
use crate::api::Attribution;
use crate::carts::{Delivery, Owner};
use crate::errors;
use crate::service::OrdersService;

/// Header mang token của giỏ vãng lai.
pub(crate) const CART_TOKEN: &str = "cart-token";

fn owner(principal: Option<&Principal>, headers: &HeaderMap) -> Result<Owner, Error> {
    let token = headers.get(CART_TOKEN).and_then(|value| value.to_str().ok());
    Owner::of(principal, token).ok_or_else(errors::cart_token_invalid)
}

/// A new cart.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = CreateOrdersCartRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCartRequest {
    /// The shop (or marketplace store) selling; omitted: the root unit of the tenant.
    org_id: Option<Uuid>,
    /// ISO 4217 code; default `VND`.
    #[schema(example = "VND")]
    currency: Option<String>,
    /// Language of product names and promotion terms; default `vi`.
    #[schema(example = "vi")]
    locale: Option<String>,
}

/// Create a cart.
///
/// Public. Signed in: your cart at this shop (an existing active cart is returned with 200).
/// Guest: the answer carries `cart_token` once — send it back in the `Cart-Token` header for
/// every call on this cart.
#[utoipa::path(
    post,
    path = "/carts",
    extensions(("x-public" = json!(true))),
    operation_id = "create_orders_cart",
    security((), ("bearer" = [])),
    request_body = CreateCartRequest,
    responses(
        (status = 201, description = "Created.", body = Data<CartResponse>),
        (status = 200, description = "You already have an active cart at this shop; here it is.", body = Data<CartResponse>),
        (status = 404, description = "`org_id` is not a unit of this shop; code `org_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), like an unknown currency (`money.currency_unknown`).", body = Problem),
    )
)]
pub(crate) async fn create_cart(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    ValidJson(input): ValidJson<CreateCartRequest>,
) -> Result<Response, Error> {
    let created = service
        .create_cart(
            &tenant,
            principal.as_ref(),
            input.org_id,
            input.currency.as_deref(),
            input.locale.as_deref(),
        )
        .await?;
    Ok(match created {
        Submitted::Created(new) => {
            let location = format!("/orders/carts/{}", new.view.cart.id);
            Created::new(location, CartResponse::new(new.view, new.token)).into_response()
        }
        Submitted::Replayed(new) => {
            Data { data: CartResponse::new(new.view, None) }.into_response()
        }
    })
}

/// Read a cart with its quote.
///
/// Signed-in owner, or guest with the `Cart-Token` header. Prices, discounts, shipping fees and
/// taxes are computed again by the server on every read; numbers sent by clients are never used.
#[utoipa::path(
    get,
    path = "/carts/{id}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_orders_cart",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the cart."), ("Cart-Token" = Option<String>, Header, description = "Token of a guest cart.")),
    responses(
        (status = 200, description = "The cart and its quote.", body = Data<CartResponse>),
        (status = 401, description = "Guest cart without the right `Cart-Token`; code `orders.cart_token_invalid`.", body = Problem),
        (status = 404, description = "No such cart of yours, or it expired; code `orders.cart_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_cart(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Data<CartResponse>, Error> {
    let owner = owner(principal.as_ref(), &headers)?;
    let row = service.owned_cart(&tenant, id, &owner).await?;
    Ok(Data { data: CartResponse::new(service.view(&tenant, row, &owner).await?, None) })
}

/// The quantity of one product in the cart.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SetOrdersCartLineRequest)]
// Không `deny_unknown_fields`: giá / tổng client gửi kèm bị bỏ qua (đặc tả 45), không làm lỗi.
pub(crate) struct SetLineRequest {
    /// `0` removes the line.
    quantity: i64,
}

/// Set the quantity of a product in the cart.
///
/// Signed-in owner, or guest with `Cart-Token`. Only the quantity is taken: prices are always the
/// server's. Stock is checked but not held until checkout.
#[utoipa::path(
    put,
    path = "/carts/{id}/lines/{variant_id}",
    extensions(("x-public" = json!(true))),
    operation_id = "set_orders_cart_line",
    security((), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the cart."),
        ("variant_id" = Uuid, Path, description = "Id of the product variant."),
        ("Cart-Token" = Option<String>, Header, description = "Token of a guest cart."),
    ),
    request_body = SetLineRequest,
    responses(
        (status = 200, description = "The cart, priced again.", body = Data<CartResponse>),
        (status = 401, description = "Guest cart without the right `Cart-Token`; code `orders.cart_token_invalid`.", body = Problem),
        (status = 404, description = "No such cart; code `orders.cart_not_found`.", body = Problem),
        (status = 409, description = "Not enough in stock (`orders.out_of_stock`, with `available`), pre-orders closed (`orders.preorder_closed`), or the cart was merged / ordered (`orders.cart_closed`).", body = Problem),
        (status = 422, description = "Quantity out of range (`orders.quantity_invalid`), too many products (`orders.cart_too_many_lines`), or the product is not on sale (`orders.variant_unavailable`).", body = Problem),
    )
)]
pub(crate) async fn set_line(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    headers: HeaderMap,
    Path((id, variant)): Path<(Uuid, Uuid)>,
    ValidJson(input): ValidJson<SetLineRequest>,
) -> Result<Data<CartResponse>, Error> {
    let owner = owner(principal.as_ref(), &headers)?;
    let view = service.set_line(&tenant, id, &owner, variant, input.quantity).await?;
    Ok(Data { data: CartResponse::new(view, None) })
}

/// Coupon codes of the cart.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SetOrdersCartCodesRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CodesRequest {
    /// Replaces the codes of the cart; `[]` removes them all.
    codes: Vec<String>,
}

/// Set the coupon codes of the cart.
///
/// Signed-in owner, or guest with `Cart-Token`. A code that does not apply does not fail: the
/// cart lists it in `rejected_codes` with the reason.
#[utoipa::path(
    put,
    path = "/carts/{id}/codes",
    extensions(("x-public" = json!(true))),
    operation_id = "set_orders_cart_codes",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the cart."), ("Cart-Token" = Option<String>, Header, description = "Token of a guest cart.")),
    request_body = CodesRequest,
    responses(
        (status = 200, description = "The cart, priced again.", body = Data<CartResponse>),
        (status = 401, description = "Guest cart without the right `Cart-Token`; code `orders.cart_token_invalid`.", body = Problem),
        (status = 404, description = "No such cart; code `orders.cart_not_found`.", body = Problem),
        (status = 409, description = "The cart was merged or ordered; code `orders.cart_closed`.", body = Problem),
        (status = 422, description = "A code is empty or too long (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn set_codes(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CodesRequest>,
) -> Result<Data<CartResponse>, Error> {
    let owner = owner(principal.as_ref(), &headers)?;
    Ok(Data {
        data: CartResponse::new(service.set_codes(&tenant, id, &owner, input.codes).await?, None),
    })
}

/// Vouchers of the cart.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SetOrdersCartVouchersRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct VouchersRequest {
    /// Ids of vouchers in your wallet; replaces the list, `[]` removes them all.
    voucher_ids: Vec<Uuid>,
}

/// Use vouchers of your wallet for the cart.
///
/// Signed-in owner only (vouchers live in an account's wallet). A voucher that does not apply is
/// listed in `rejected_codes` with the reason.
#[utoipa::path(
    put,
    path = "/carts/{id}/vouchers",
    extensions(("x-permissions" = json!([]))),
    operation_id = "set_orders_cart_vouchers",
    params(("id" = Uuid, Path, description = "Id of the cart.")),
    request_body = VouchersRequest,
    responses(
        (status = 200, description = "The cart, priced again.", body = Data<CartResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such cart of yours; code `orders.cart_not_found`.", body = Problem),
        (status = 409, description = "The cart was merged or ordered; code `orders.cart_closed`.", body = Problem),
    )
)]
pub(crate) async fn set_vouchers(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<VouchersRequest>,
) -> Result<Data<CartResponse>, Error> {
    let owner = Owner::Buyer(principal.user_id());
    let view = service.set_vouchers(&tenant, id, &owner, input.voucher_ids).await?;
    Ok(Data { data: CartResponse::new(view, None) })
}

/// Delivery of the cart.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = SetOrdersCartDeliveryRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeliveryRequest {
    /// An address of your address book (signed-in buyers).
    address_id: Option<Uuid>,
    /// An address typed in (guests, or a one-off address); names are filled in.
    address: Option<autoxide_address::api::AddressInput>,
    #[validate(length(min = 1, max = 200))]
    recipient_name: Option<String>,
    #[validate(length(min = 6, max = 20))]
    phone: Option<String>,
    /// `option_key` of one of the cart's `shipping.options`.
    #[validate(length(min = 1, max = 200))]
    shipping_option: Option<String>,
}

/// Set where and how the cart is delivered.
///
/// Signed-in owner, or guest with `Cart-Token`. The cart lists the delivery options for the
/// address in `shipping.options`; choose one by its `option_key`.
#[utoipa::path(
    put,
    path = "/carts/{id}/delivery",
    extensions(("x-public" = json!(true))),
    operation_id = "set_orders_cart_delivery",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the cart."), ("Cart-Token" = Option<String>, Header, description = "Token of a guest cart.")),
    request_body = DeliveryRequest,
    responses(
        (status = 200, description = "The cart, priced again.", body = Data<CartResponse>),
        (status = 401, description = "Guest cart without the right `Cart-Token`; code `orders.cart_token_invalid`.", body = Problem),
        (status = 404, description = "No such cart (`orders.cart_not_found`), or no such address in your book (`shipping.address_not_found`).", body = Problem),
        (status = 409, description = "The cart was merged or ordered; code `orders.cart_closed`.", body = Problem),
        (status = 422, description = "The address is invalid, or both `address_id` and `address` were sent (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn set_delivery(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<DeliveryRequest>,
) -> Result<Data<CartResponse>, Error> {
    let owner = owner(principal.as_ref(), &headers)?;
    let delivery = Delivery {
        address_id: input.address_id,
        address: input.address,
        recipient_name: input.recipient_name,
        phone: input.phone,
        shipping_option: input.shipping_option,
    };
    Ok(Data {
        data: CartResponse::new(service.set_delivery(&tenant, id, &owner, delivery).await?, None),
    })
}

/// Payment method of the cart.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SetOrdersCartPaymentMethodRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaymentMethodRequest {
    /// `gateway`, `vietqr`, `bank_transfer`, `cod`…; `null` clears it.
    #[validate(length(min = 1, max = 32))]
    payment_method: Option<String>,
}

/// Choose the payment method of the cart.
///
/// Signed-in owner, or guest with `Cart-Token`. Some promotions depend on the payment method, so
/// the cart is priced again.
#[utoipa::path(
    put,
    path = "/carts/{id}/payment-method",
    extensions(("x-public" = json!(true))),
    operation_id = "set_orders_cart_payment_method",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the cart."), ("Cart-Token" = Option<String>, Header, description = "Token of a guest cart.")),
    request_body = PaymentMethodRequest,
    responses(
        (status = 200, description = "The cart, priced again.", body = Data<CartResponse>),
        (status = 401, description = "Guest cart without the right `Cart-Token`; code `orders.cart_token_invalid`.", body = Problem),
        (status = 404, description = "No such cart; code `orders.cart_not_found`.", body = Problem),
        (status = 409, description = "The cart was merged or ordered; code `orders.cart_closed`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn set_payment_method(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<PaymentMethodRequest>,
) -> Result<Data<CartResponse>, Error> {
    let owner = owner(principal.as_ref(), &headers)?;
    let view = service.set_payment_method(&tenant, id, &owner, input.payment_method).await?;
    Ok(Data { data: CartResponse::new(view, None) })
}

/// Referral of the cart.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SetOrdersCartAttributionRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttributionRequest {
    /// Affiliate click id.
    click_id: Option<Uuid>,
    /// Affiliate or referral code.
    #[validate(length(min = 1, max = 64))]
    code: Option<String>,
    /// Live-selling session.
    live_session_id: Option<Uuid>,
}

/// Record who referred the buyer to the cart.
///
/// Signed-in owner, or guest with `Cart-Token`. The order keeps it for commissions.
#[utoipa::path(
    put,
    path = "/carts/{id}/attribution",
    extensions(("x-public" = json!(true))),
    operation_id = "set_orders_cart_attribution",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the cart."), ("Cart-Token" = Option<String>, Header, description = "Token of a guest cart.")),
    request_body = AttributionRequest,
    responses(
        (status = 200, description = "The cart.", body = Data<CartResponse>),
        (status = 401, description = "Guest cart without the right `Cart-Token`; code `orders.cart_token_invalid`.", body = Problem),
        (status = 404, description = "No such cart; code `orders.cart_not_found`.", body = Problem),
        (status = 409, description = "The cart was merged or ordered; code `orders.cart_closed`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn set_attribution(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AttributionRequest>,
) -> Result<Data<CartResponse>, Error> {
    let owner = owner(principal.as_ref(), &headers)?;
    let attribution = Attribution::new(input.click_id, input.code, input.live_session_id);
    let view = service.set_attribution(&tenant, id, &owner, attribution).await?;
    Ok(Data { data: CartResponse::new(view, None) })
}

/// Codes and vouchers usable for the cart.
///
/// Signed-in owner, or guest with `Cart-Token`. Lists usable offers, offers not reached yet with
/// the reason, and the best combination. Empty when promotions are not installed.
#[utoipa::path(
    get,
    path = "/carts/{id}/promotions",
    extensions(("x-public" = json!(true))),
    operation_id = "list_orders_cart_promotions",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the cart."), ("Cart-Token" = Option<String>, Header, description = "Token of a guest cart.")),
    responses(
        (status = 200, description = "Offers for the cart.", body = Data<Option<Eligibility>>),
        (status = 401, description = "Guest cart without the right `Cart-Token`; code `orders.cart_token_invalid`.", body = Problem),
        (status = 404, description = "No such cart; code `orders.cart_not_found`.", body = Problem),
    )
)]
pub(crate) async fn cart_promotions(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Data<Option<Eligibility>>, Error> {
    let owner = owner(principal.as_ref(), &headers)?;
    Ok(Data { data: service.cart_promotions(&tenant, id, &owner).await? })
}

/// Merge a guest cart into your cart.
///
/// Signed in, with the guest cart's `Cart-Token` header (call it right after signing in).
/// Quantities add up (capped at the maximum), coupon codes are kept; the guest cart becomes
/// `merged`. Calling again returns your cart without adding twice.
#[utoipa::path(
    post,
    path = "/carts/{id}/merge",
    extensions(("x-permissions" = json!([]))),
    operation_id = "merge_orders_cart",
    params(("id" = Uuid, Path, description = "Id of the guest cart."), ("Cart-Token" = String, Header, description = "Token of the guest cart.")),
    responses(
        (status = 200, description = "Your cart after the merge.", body = Data<CartResponse>),
        (status = 401, description = "Not signed in (`unauthenticated`), or wrong `Cart-Token` (`orders.cart_token_invalid`).", body = Problem),
        (status = 404, description = "No such guest cart; code `orders.cart_not_found`.", body = Problem),
    )
)]
pub(crate) async fn merge_cart(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Data<CartResponse>, Error> {
    let token = headers
        .get(CART_TOKEN)
        .and_then(|value| value.to_str().ok())
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(errors::cart_token_invalid)?;
    let view = service.merge(&tenant, principal.user_id(), id, token.trim()).await?;
    Ok(Data { data: CartResponse::new(view, None) })
}

/// Filter of the staff cart list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct CartListQuery {
    /// `abandoned` (default), `active`, `merged` or `converted`.
    status: Option<String>,
    /// Only carts of this shop.
    org_id: Option<Uuid>,
}

/// List carts, like abandoned carts.
///
/// Requires `orders.read` (carts of shops in your scope).
#[utoipa::path(
    get,
    path = "/carts",
    extensions(("x-permissions" = json!(["orders.read"]))),
    operation_id = "list_orders_carts",
    params(CartListQuery, PageQuery),
    responses(
        (status = 200, description = "A page of carts, newest first by default.", body = Page<CartSummaryResponse>),
        (status = 400, description = "Bad cursor or limit.", body = Problem),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Unknown `status`; code `validation_failed`.", body = Problem),
    )
)]
pub(crate) async fn list_carts(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<CartListQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<CartSummaryResponse>, Error> {
    let status = query.status.as_deref().unwrap_or(crate::carts::ABANDONED);
    if ![
        crate::carts::ABANDONED,
        crate::carts::ACTIVE,
        crate::carts::MERGED,
        crate::carts::CONVERTED,
    ]
    .contains(&status)
    {
        return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
            "status",
            "invalid",
            "must be one of `abandoned`, `active`, `merged`, `converted`",
        )]));
    }
    let page = PageRequest::try_from(page)?;
    let found = service.list_carts(&tenant, &principal, status, query.org_id, &page).await?;
    Ok(found.map(CartSummaryResponse::from))
}
