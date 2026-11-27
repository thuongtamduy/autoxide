//! Endpoint mua không cần tài khoản (D-201, D-202, D-204): tra đơn bằng số đơn + mã một lần, token
//! xem đơn (header `Order-Token`), huỷ / đổi địa chỉ / mở hỗ trợ bằng token, gắn đơn vãng lai vào
//! tài khoản, mở hỗ trợ cho đơn của người mua có tài khoản.
//!
//! Token đi bằng header riêng `Order-Token` (như `Cart-Token`), không bằng `Authorization`:
//! `Authorization` là của đăng nhập — kernel xác thực mọi giá trị trong đó và trả 401 nếu không phải
//! token đăng nhập (D-20261120-step45-guest-orders).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_address::api::AddressInput;
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::dto::OrderResponse;
use crate::guests::{
    AccessIssued, AddressChange, CodeSent, GuestAccess, ORDER_TOKEN, mask_name, mask_phone,
    mask_ship_to,
};
use crate::service::OrdersService;

/// `Idempotency-Key` của request (không bắt buộc với thao tác vãng lai).
pub(super) fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|key| !key.is_empty() && key.len() <= 200)
        .map(str::to_owned)
}

pub(super) async fn access(
    service: &OrdersService,
    tenant: &TenantContext,
    headers: &HeaderMap,
) -> Result<GuestAccess, Error> {
    let token = headers.get(ORDER_TOKEN).and_then(|value| value.to_str().ok());
    service.guest_access(tenant, token).await
}

/// Look up a guest order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersLookupRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct LookupRequest {
    /// Order number, as on the confirmation.
    #[schema(example = "2026-000123")]
    number: String,
    /// Phone number given when ordering.
    #[schema(example = "0901234567")]
    phone: String,
}

/// A one-time code was sent by SMS.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersCodeSent)]
pub(crate) struct CodeSentResponse {
    /// Id of the code: confirm with it.
    lookup_id: Uuid,
    order_id: Uuid,
    /// The code works until then.
    expires_at: Timestamp,
    /// Seconds to wait before asking for another code.
    resend_after: i64,
    /// Where the code went, masked (`090•••567`).
    phone: String,
}

impl From<CodeSent> for CodeSentResponse {
    fn from(sent: CodeSent) -> Self {
        CodeSentResponse {
            lookup_id: sent.id,
            order_id: sent.order_id,
            expires_at: sent.expires_at,
            resend_after: sent.resend_after,
            phone: sent.masked_phone,
        }
    }
}

/// Look up a guest order by number and phone.
///
/// Public. Sends a 6-digit code by SMS to the phone number of the order; confirm it with
/// `POST /public/orders/lookup/{lookup_id}/confirm`. A wrong phone number and an unknown order
/// number get the same answer. Limited per order number and phone number
/// (`AUTOXIDE_ORDERS_LOOKUP_PER_HOUR`) and per client address.
#[utoipa::path(
    post,
    path = "/public/orders/lookup",
    extensions(("x-public" = json!(true))),
    operation_id = "lookup_guest_order",
    security(()),
    request_body = LookupRequest,
    responses(
        (status = 200, description = "Code sent.", body = Data<CodeSentResponse>),
        (status = 404, description = "No order with this number and phone; code `orders.lookup_failed`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
        (status = 429, description = "Too many lookups (`orders.lookup_too_many`) or a code was sent too recently (`orders.lookup_resend_too_soon`); see `Retry-After`.", body = Problem),
        (status = 503, description = "Sending SMS is not set up; code `sms_disabled`.", body = Problem),
    )
)]
pub(crate) async fn lookup(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    client: ClientInfo,
    ValidJson(input): ValidJson<LookupRequest>,
) -> Result<Data<CodeSentResponse>, Error> {
    let sent = service.start_lookup(&tenant, &input.number, &input.phone, client.ip).await?;
    Ok(Data { data: sent.into() })
}

/// The one-time code received by SMS.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersLookupConfirmRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfirmRequest {
    #[validate(length(min = 1, max = 12))]
    #[schema(example = "123456")]
    code: String,
}

/// Access to one guest order.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersAccessToken)]
pub(crate) struct AccessResponse {
    /// Send it in the `Order-Token` header to see and act on this one order. Returned once.
    access_token: String,
    order_id: Uuid,
    expires_at: Timestamp,
}

impl From<AccessIssued> for AccessResponse {
    fn from(issued: AccessIssued) -> Self {
        AccessResponse {
            access_token: issued.token,
            order_id: issued.order_id,
            expires_at: issued.expires_at,
        }
    }
}

/// Confirm a guest order lookup with the code.
///
/// Public. A right code gives an access token for this one order (valid for
/// `AUTOXIDE_ORDERS_LOOKUP_TOKEN_MINUTES`). A code works once and fails after 5 wrong tries.
#[utoipa::path(
    post,
    path = "/public/orders/lookup/{id}/confirm",
    extensions(("x-public" = json!(true))),
    operation_id = "confirm_guest_order_lookup",
    security(()),
    params(("id" = Uuid, Path, description = "`lookup_id` of the code sent.")),
    request_body = ConfirmRequest,
    responses(
        (status = 200, description = "The access token.", body = Data<AccessResponse>),
        (status = 422, description = "Wrong, used or expired code; code `orders.lookup_code_invalid`.", body = Problem),
    )
)]
pub(crate) async fn confirm_lookup(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ConfirmRequest>,
) -> Result<Data<AccessResponse>, Error> {
    let issued = service.confirm_lookup(&tenant, id, &input.code).await?;
    Ok(Data { data: issued.into() })
}

/// Contact of a guest order, masked.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersMaskedContact)]
pub(crate) struct MaskedContactResponse {
    /// Initials of family and middle names, then the given name (`N. V. An`).
    name: String,
    /// First 3 and last 3 digits (`090•••567`).
    phone: String,
}

/// An order seen with an `Order-Token`: personal data is masked.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersGuestOrder)]
pub(crate) struct GuestOrderResponse {
    #[serde(flatten)]
    order: OrderResponse,
    contact: Option<MaskedContactResponse>,
    /// Delivery: recipient masked, address down to the commune / ward (no house number or street).
    #[schema(value_type = Option<Object>)]
    ship_to: Option<Value>,
}

impl GuestOrderResponse {
    fn new(view: crate::orders_read::OrderView) -> Result<Self, Error> {
        let contact = serde_json::from_str::<crate::checkout::Contact>(&view.order.contact)
            .ok()
            .map(|contact| MaskedContactResponse {
                name: mask_name(&contact.name),
                phone: mask_phone(&contact.phone),
            });
        let ship_to = view.order.ship_to.as_deref().and_then(mask_ship_to);
        Ok(GuestOrderResponse { order: OrderResponse::for_buyer(view)?, contact, ship_to })
    }
}

async fn guest_view(
    service: &OrdersService,
    tenant: &TenantContext,
    row: crate::entity::order::Model,
) -> Result<Data<GuestOrderResponse>, Error> {
    let view = service.order_view(tenant, row).await?;
    Ok(Data { data: GuestOrderResponse::new(view)? })
}

/// See a guest order.
///
/// Public with the `Order-Token` header: only the order the token opens. Phone, name and address
/// are masked.
#[utoipa::path(
    get,
    path = "/public/orders/{id}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_guest_order",
    security(()),
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("Order-Token" = String, Header, description = "Access token of the order."),
    ),
    responses(
        (status = 200, description = "The order.", body = Data<GuestOrderResponse>),
        (status = 401, description = "Missing, wrong or expired token; code `orders.lookup_token_invalid`.", body = Problem),
        (status = 404, description = "The token opens another order; code `orders.order_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_guest_order(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Data<GuestOrderResponse>, Error> {
    let access = access(&service, &tenant, &headers).await?;
    let row = service.guest_order(&tenant, &access, id).await?;
    guest_view(&service, &tenant, row).await
}

/// Cancel a guest order.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersGuestCancelRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuestCancelRequest {
    #[validate(length(min = 1, max = 200))]
    reason: Option<String>,
}

/// Cancel a guest order.
///
/// Public with the `Order-Token` header. Same rule as for signed-in buyers: allowed until the
/// status the shop's policy says (`orders.buyer_cancel_until`); a paid order is refunded through
/// the payment partner. Sending the same `Idempotency-Key` again returns the order.
#[utoipa::path(
    post,
    path = "/public/orders/{id}/cancel",
    extensions(("x-public" = json!(true))),
    operation_id = "cancel_guest_order",
    security(()),
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("Order-Token" = String, Header, description = "Access token of the order."),
        ("Idempotency-Key" = Option<String>, Header, description = "Key of this attempt; resend the same key when retrying."),
    ),
    request_body = GuestCancelRequest,
    responses(
        (status = 200, description = "The cancelled order.", body = Data<GuestOrderResponse>, headers(("Idempotent-Replayed" = String, description = "`true` when the same `Idempotency-Key` was sent before: the current state; nothing was done again."))),
        (status = 401, description = "Missing, wrong or expired token; code `orders.lookup_token_invalid`.", body = Problem),
        (status = 404, description = "The token opens another order; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "Too late to cancel by yourself; code `orders.cancel_not_allowed`.", body = Problem),
        (status = 422, description = "Same `Idempotency-Key` with another body (`orders.idempotency_key_reused`) or invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn cancel_guest_order(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<GuestCancelRequest>,
) -> Result<Response, Error> {
    let access = access(&service, &tenant, &headers).await?;
    let key = idempotency_key(&headers).unwrap_or_else(|| Uuid::now_v7().to_string());
    let submitted = service.guest_cancel(&tenant, &access, id, input.reason, &key).await?;
    let (replayed, row) = super::replay::split(submitted);
    Ok(super::replay::marked(guest_view(&service, &tenant, row).await?, replayed))
}

/// Ask for a code to change the delivery address.
///
/// Public with the `Order-Token` header. Sends a second one-time code by SMS to the phone number of
/// the order; only this token can use it. Only while the goods have not left the warehouse.
#[utoipa::path(
    post,
    path = "/public/orders/{id}/address/otp",
    extensions(("x-public" = json!(true))),
    operation_id = "send_guest_address_code",
    security(()),
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("Order-Token" = String, Header, description = "Access token of the order."),
    ),
    responses(
        (status = 200, description = "Code sent.", body = Data<CodeSentResponse>),
        (status = 401, description = "Missing, wrong or expired token; code `orders.lookup_token_invalid`.", body = Problem),
        (status = 404, description = "The token opens another order; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "The goods left the warehouse or the order is collected in store; code `orders.address_locked`.", body = Problem),
        (status = 429, description = "A code was sent too recently; code `orders.lookup_resend_too_soon`, see `Retry-After`.", body = Problem),
        (status = 503, description = "Sending SMS is not set up; code `sms_disabled`.", body = Problem),
    )
)]
pub(crate) async fn send_address_code(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Data<CodeSentResponse>, Error> {
    let access = access(&service, &tenant, &headers).await?;
    let sent = service.guest_address_code(&tenant, &access, id).await?;
    Ok(Data { data: sent.into() })
}

/// A new delivery address.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersGuestAddressRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddressRequest {
    address: AddressInput,
    /// New recipient; omitted: unchanged.
    #[validate(length(min = 1, max = 200))]
    recipient_name: Option<String>,
    /// New phone of the recipient; omitted: unchanged.
    #[validate(length(min = 8, max = 20))]
    phone: Option<String>,
    /// The code from `POST /public/orders/{id}/address/otp`.
    #[validate(length(min = 1, max = 12))]
    code: String,
}

/// Change the delivery address of a guest order.
///
/// Public with the `Order-Token` header and the second code. Only before the goods leave the
/// warehouse (no shipment requested yet). The shipping fee is computed again for the same shipping
/// method: when it changes, nothing changes and the answer is `orders.address_change_needs_payment`.
#[utoipa::path(
    post,
    path = "/public/orders/{id}/address",
    extensions(("x-public" = json!(true))),
    operation_id = "change_guest_order_address",
    security(()),
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("Order-Token" = String, Header, description = "Access token of the order."),
        ("Idempotency-Key" = Option<String>, Header, description = "Key of this attempt; resend the same key when retrying."),
    ),
    request_body = AddressRequest,
    responses(
        (status = 200, description = "The order with the new address.", body = Data<GuestOrderResponse>, headers(("Idempotent-Replayed" = String, description = "`true` when the same `Idempotency-Key` was sent before: the current state; nothing was done again."))),
        (status = 401, description = "Missing, wrong or expired token; code `orders.lookup_token_invalid`.", body = Problem),
        (status = 404, description = "The token opens another order; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "The goods left the warehouse (`orders.address_locked`) or the fee changes (`orders.address_change_needs_payment`).", body = Problem),
        (status = 422, description = "Wrong, used or expired code (`orders.lookup_code_invalid`), same `Idempotency-Key` with another body (`orders.idempotency_key_reused`), or invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn change_address(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AddressRequest>,
) -> Result<Response, Error> {
    let access = access(&service, &tenant, &headers).await?;
    let key = idempotency_key(&headers).unwrap_or_else(|| Uuid::now_v7().to_string());
    let change = AddressChange {
        address: input.address,
        recipient_name: input.recipient_name,
        phone: input.phone,
        code: input.code,
    };
    let submitted = service.guest_change_address(&tenant, &access, id, change, &key).await?;
    let (replayed, row) = super::replay::split(submitted);
    Ok(super::replay::marked(guest_view(&service, &tenant, row).await?, replayed))
}

/// A message to customer support about an order.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersSupportRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportRequest {
    #[validate(length(min = 1, max = 5000))]
    message: String,
}

/// The support ticket opened.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersSupportTicket)]
pub(crate) struct SupportTicketResponse {
    ticket_id: Uuid,
    /// Number shown to the buyer.
    number: String,
    /// Guest: token to follow the ticket at `support`; only on the first answer.
    access_token: Option<String>,
}

impl From<autoxide_support::api::TicketRef> for SupportTicketResponse {
    fn from(ticket: autoxide_support::api::TicketRef) -> Self {
        SupportTicketResponse {
            ticket_id: ticket.id,
            number: ticket.number,
            access_token: ticket.access_token,
        }
    }
}

/// Open a support ticket about a guest order.
///
/// Public with the `Order-Token` header. The ticket is linked to the order; the asker is the phone
/// number proved by the code. Sending the same `Idempotency-Key` again returns the same ticket.
#[utoipa::path(
    post,
    path = "/public/orders/{id}/support",
    extensions(("x-public" = json!(true))),
    operation_id = "open_guest_order_support",
    security(()),
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("Order-Token" = String, Header, description = "Access token of the order."),
        ("Idempotency-Key" = Option<String>, Header, description = "Key of this attempt; resend the same key when retrying."),
    ),
    request_body = SupportRequest,
    responses(
        (status = 201, description = "Ticket opened (or the one of the same key).", body = Data<SupportTicketResponse>),
        (status = 401, description = "Missing, wrong or expired token; code `orders.lookup_token_invalid`.", body = Problem),
        (status = 404, description = "The token opens another order (`orders.order_not_found`) or the shop has no support (`orders.support_unavailable`).", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn guest_support(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SupportRequest>,
) -> Result<Created<SupportTicketResponse>, Error> {
    let access = access(&service, &tenant, &headers).await?;
    let key = idempotency_key(&headers).unwrap_or_else(|| Uuid::now_v7().to_string());
    let ticket = service.guest_support(&tenant, &access, id, &input.message, &key).await?;
    let path = format!("/support/tickets/{}", ticket.id);
    Ok(Created::new(path, ticket.into()))
}

/// Open a support ticket about one of your orders.
///
/// Signed in: an order you placed. The ticket is linked to the order. Sending the same
/// `Idempotency-Key` again returns the same ticket.
#[utoipa::path(
    post,
    path = "/me/orders/{id}/support",
    extensions(("x-permissions" = json!([]))),
    operation_id = "open_my_order_support",
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("Idempotency-Key" = Option<String>, Header, description = "Key of this attempt; resend the same key when retrying."),
    ),
    request_body = SupportRequest,
    responses(
        (status = 201, description = "Ticket opened (or the one of the same key).", body = Data<SupportTicketResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such order of yours (`orders.order_not_found`) or the shop has no support (`orders.support_unavailable`).", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn my_order_support(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SupportRequest>,
) -> Result<Created<SupportTicketResponse>, Error> {
    let key = idempotency_key(&headers).unwrap_or_else(|| Uuid::now_v7().to_string());
    let ticket =
        service.buyer_support(&tenant, principal.user_id(), id, &input.message, &key).await?;
    let path = format!("/support/tickets/{}", ticket.id);
    Ok(Created::new(path, ticket.into()))
}

/// Attach guest orders to your account.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersClaimRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimRequest {
    /// An `access_token` of a guest order lookup: it proves you own the phone number.
    #[validate(length(min = 1, max = 200))]
    lookup_token: String,
}

/// Guest orders attached to your account.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersClaimed)]
pub(crate) struct ClaimResponse {
    order_ids: Vec<Uuid>,
}

/// Attach your guest orders to your account.
///
/// Signed in. The lookup token proves the phone number (by the one-time code); every guest order
/// of this shop with that phone number becomes yours. Calling again changes nothing more.
#[utoipa::path(
    post,
    path = "/me/orders/claim",
    extensions(("x-permissions" = json!([]))),
    operation_id = "claim_guest_orders",
    request_body = ClaimRequest,
    responses(
        (status = 200, description = "The orders attached (empty when none was left).", body = Data<ClaimResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 422, description = "Wrong or expired `lookup_token` (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn claim(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ClaimRequest>,
) -> Result<Data<ClaimResponse>, Error> {
    let order_ids = service.claim_orders(&tenant, principal.user_id(), &input.lookup_token).await?;
    Ok(Data { data: ClaimResponse { order_ids } })
}
