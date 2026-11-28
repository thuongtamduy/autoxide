//! Endpoint bán tại quầy (POS) cho chuỗi (D-227, D-340): ca, bán, tạm tính, tra mã vạch. Handler
//! chỉ lấy dữ liệu vào → gọi service → trả envelope.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::dto::{OrderResponse, PosQuoteResponse};
use crate::entity::pos_shift;
use crate::pos::{PosCart, PosLine, PosPayment};
use crate::service::OrdersService;

/// A cash register shift.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersPosShift)]
pub(crate) struct ShiftResponse {
    id: Uuid,
    /// The store.
    org_id: Uuid,
    register_code: String,
    /// `open`, `pending_approval` (cash difference above the threshold) or `closed`.
    status: String,
    opened_by: Uuid,
    opened_at: Timestamp,
    opening_cash: Money,
    closed_by: Option<Uuid>,
    closed_at: Option<Timestamp>,
    /// Opening cash plus cash taken by sales of the shift.
    expected_cash: Option<Money>,
    counted_cash: Option<Money>,
    /// `counted − expected`: negative is a shortage.
    variance: Option<Money>,
    /// Code of the list `orders.cash_variance`.
    variance_reason: Option<String>,
    variance_note: Option<String>,
    approved_by: Option<Uuid>,
    approved_at: Option<Timestamp>,
}

impl ShiftResponse {
    fn new(row: pos_shift::Model) -> Result<Self, Error> {
        let money = |minor: i64| Money::from_columns(minor, &row.currency);
        Ok(ShiftResponse {
            id: row.id,
            org_id: row.org_id,
            register_code: row.register_code.clone(),
            status: row.status.clone(),
            opened_by: row.opened_by,
            opened_at: row.opened_at,
            opening_cash: money(row.opening_cash_minor)?,
            closed_by: row.closed_by,
            closed_at: row.closed_at,
            expected_cash: row.expected_cash_minor.map(money).transpose()?,
            counted_cash: row.counted_cash_minor.map(money).transpose()?,
            variance: row.variance_minor.map(money).transpose()?,
            variance_reason: row.variance_reason.clone(),
            variance_note: row.variance_note.clone(),
            approved_by: row.approved_by,
            approved_at: row.approved_at,
        })
    }
}

/// Open a shift.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersOpenShiftRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenShiftRequest {
    /// The store.
    org_id: Uuid,
    /// Code of the register (one open shift per register).
    #[validate(length(min = 1, max = 40))]
    #[schema(example = "Q1")]
    register_code: String,
    /// Cash in the drawer at the start.
    opening_cash: Money,
}

/// Open a cash register shift.
///
/// Requires `orders.pos.shift` at the store. One open shift per register.
#[utoipa::path(
    post,
    path = "/pos/shifts",
    extensions(("x-permissions" = json!(["orders.pos.shift"]))),
    operation_id = "open_pos_shift",
    request_body = OpenShiftRequest,
    responses(
        (status = 201, description = "The shift.", body = Data<ShiftResponse>, headers(("Location" = String, description = "Path of the shift."))),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.pos.shift` at the store; code `permission_denied`.", body = Problem),
        (status = 409, description = "The register has an open shift; code `orders.shift_already_open`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn open_shift(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<OpenShiftRequest>,
) -> Result<Created<ShiftResponse>, Error> {
    let org = OrgId::from_uuid(input.org_id);
    let row = service
        .open_shift(&tenant, &principal, org, &input.register_code, input.opening_cash)
        .await?;
    let path = format!("/orders/pos/shifts/{}", row.id);
    Ok(Created::new(path, ShiftResponse::new(row)?))
}

/// Close a shift.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersCloseShiftRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloseShiftRequest {
    /// Cash counted in the drawer.
    counted_cash: Money,
    /// Required when the count differs: a code of the list `orders.cash_variance`.
    #[validate(length(min = 1, max = 64))]
    variance_reason: Option<String>,
    /// Required when the count differs.
    #[validate(length(min = 1, max = 1000))]
    variance_note: Option<String>,
}

/// Close a cash register shift.
///
/// Requires `orders.pos.shift` at the store. The difference between counted and expected cash
/// needs a reason and a note; it is booked at once to the store's suspense account. Above the
/// store's threshold (`orders.cash_variance_approval_minor`) the shift waits for someone else to
/// approve it.
#[utoipa::path(
    post,
    path = "/pos/shifts/{id}/close",
    extensions(("x-permissions" = json!(["orders.pos.shift"]))),
    operation_id = "close_pos_shift",
    params(("id" = Uuid, Path, description = "Id of the shift.")),
    request_body = CloseShiftRequest,
    responses(
        (status = 200, description = "The shift, `closed` or `pending_approval`.", body = Data<ShiftResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.pos.shift` at the store; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such shift; code `orders.shift_not_found`.", body = Problem),
        (status = 409, description = "The shift is not open; code `orders.shift_not_open`.", body = Problem),
        (status = 422, description = "Reason or note missing, `counted_cash` not in the shift currency (`orders.cash_currency_mismatch`), the cash adds up beyond the storable range (`money.overflow`), or invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn close_shift(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CloseShiftRequest>,
) -> Result<Data<ShiftResponse>, Error> {
    let row = service
        .close_shift(
            &tenant,
            &principal,
            id,
            input.counted_cash,
            input.variance_reason,
            input.variance_note,
        )
        .await?;
    Ok(Data { data: ShiftResponse::new(row)? })
}

/// Approve the cash difference of a shift.
///
/// Requires `orders.pos.shift` at the store, and being someone else than who closed the shift.
#[utoipa::path(
    post,
    path = "/pos/shifts/{id}/approve",
    extensions(("x-permissions" = json!(["orders.pos.shift"]))),
    operation_id = "approve_pos_shift",
    params(("id" = Uuid, Path, description = "Id of the shift.")),
    responses(
        (status = 200, description = "The closed shift.", body = Data<ShiftResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.pos.shift` (`permission_denied`) or you closed the shift (`orders.shift_approval_self`).", body = Problem),
        (status = 404, description = "No such shift; code `orders.shift_not_found`.", body = Problem),
        (status = 409, description = "The shift waits for no approval; code `orders.shift_not_pending`.", body = Problem),
    )
)]
pub(crate) async fn approve_shift(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ShiftResponse>, Error> {
    let row = service.approve_shift(&tenant, &principal, id).await?;
    Ok(Data { data: ShiftResponse::new(row)? })
}

/// A line sold at the counter.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersPosLine)]
#[serde(deny_unknown_fields)]
pub(crate) struct PosLineRequest {
    variant_id: Uuid,
    quantity: i64,
}

/// A payment taken at the counter.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersPosPayment)]
#[serde(deny_unknown_fields)]
pub(crate) struct PosPaymentRequest {
    /// `cash`, `card_terminal` or `vietqr`.
    method: String,
    amount: Money,
    /// Cash handed over (at least `amount`); the change is given back.
    tendered: Option<Money>,
    /// Approval code of the card terminal or id of the bank transfer.
    #[validate(length(min = 1, max = 100))]
    approval_code: Option<String>,
}

/// A cart at the counter.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersPosQuoteRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct PosQuoteRequest {
    shift_id: Uuid,
    lines: Vec<PosLineRequest>,
    /// Coupon codes; codes that do not apply are skipped and listed in `rejected_codes`.
    #[serde(default)]
    codes: Vec<String>,
    /// Customer account, for prices and promotions of their groups; empty for a walk-in customer.
    customer_id: Option<Uuid>,
}

/// A sale at the counter.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersPosSaleRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct PosSaleRequest {
    shift_id: Uuid,
    /// Unique id of the sale made by the register (also offline); sending it again returns the
    /// same sale.
    #[validate(length(min = 1, max = 200))]
    reference: String,
    lines: Vec<PosLineRequest>,
    #[serde(default)]
    codes: Vec<String>,
    customer_id: Option<Uuid>,
    /// They must add up to the order total.
    payments: Vec<PosPaymentRequest>,
}

fn cart_of(lines: Vec<PosLineRequest>, codes: Vec<String>, customer_id: Option<Uuid>) -> PosCart {
    PosCart {
        lines: lines
            .into_iter()
            .map(|line| PosLine { variant_id: line.variant_id, quantity: line.quantity })
            .collect(),
        codes,
        customer_id,
    }
}

/// Price a cart at the counter.
///
/// Requires `orders.pos.sell` at the store of the open shift. Same prices and promotions as a
/// sale; nothing is saved.
#[utoipa::path(
    post,
    path = "/pos/quote",
    extensions(("x-permissions" = json!(["orders.pos.sell"]))),
    operation_id = "quote_pos_cart",
    request_body = PosQuoteRequest,
    responses(
        (status = 200, description = "Prices, discounts, rejected codes and totals.", body = Data<PosQuoteResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.pos.sell` at the store; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such shift; code `orders.shift_not_found`.", body = Problem),
        (status = 409, description = "The shift is not open; code `orders.shift_not_open`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn quote(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PosQuoteRequest>,
) -> Result<Data<PosQuoteResponse>, Error> {
    let cart = cart_of(input.lines, input.codes, input.customer_id);
    let quote = service.pos_quote(&tenant, &principal, input.shift_id, &cart).await?;
    Ok(Data { data: PosQuoteResponse::new(&quote) })
}

/// A sale made at the counter.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersPosSale)]
pub(crate) struct PosSaleResponse {
    /// The order, `delivered` (the customer takes the goods).
    order: OrderResponse,
    /// Cash to give back.
    change: Money,
    /// Prices and rejected codes of the sale; absent when the same `reference` was sent again.
    quote: Option<PosQuoteResponse>,
}

/// Sell at the counter.
///
/// Requires `orders.pos.sell` at the store of the open shift. Stock is taken from that store only;
/// the payments must add up to the total. The order is created `delivered`; the cash register
/// invoice follows the store's invoice policy. Sending the same `reference` again returns the same
/// sale (200).
#[utoipa::path(
    post,
    path = "/pos/sales",
    extensions(("x-permissions" = json!(["orders.pos.sell"]))),
    operation_id = "create_pos_sale",
    request_body = PosSaleRequest,
    responses(
        (status = 201, description = "Sold.", body = Data<PosSaleResponse>, headers(("Location" = String, description = "Path of the order."))),
        (status = 200, description = "Same `reference` as an earlier sale of the shift: that sale.", body = Data<PosSaleResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.pos.sell` at the store; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such shift; code `orders.shift_not_found`.", body = Problem),
        (status = 409, description = "The shift is not open (`orders.shift_not_open`) or not enough stock in the store (`orders.out_of_stock`).", body = Problem),
        (status = 422, description = "Payments do not add up (`orders.payments_mismatch`) or overflow (`money.overflow`), a payment is not in the shift currency (`orders.payment_currency_mismatch`), a product not on sale (`orders.variant_unavailable`), the reference reused for another sale (`orders.idempotency_key_reused`), or invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn sell(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PosSaleRequest>,
) -> Result<Response, Error> {
    let cart = cart_of(input.lines, input.codes, input.customer_id);
    let payments = input
        .payments
        .into_iter()
        .map(|payment| {
            PosPayment::new(
                payment.method,
                payment.amount,
                payment.tendered,
                payment.approval_code,
            )
        })
        .collect();
    let sold = service
        .pos_sell(&tenant, &principal, input.shift_id, &input.reference, cart, payments)
        .await?;
    let (created, sold) = match sold {
        Submitted::Created(sold) => (true, sold),
        Submitted::Replayed(sold) => (false, sold),
    };
    let row = service.order(&tenant, sold.order_id).await?;
    let view = service.order_view(&tenant, row).await?;
    let data = PosSaleResponse {
        order: OrderResponse::new(view)?,
        change: sold.change,
        quote: sold.quote.as_ref().map(PosQuoteResponse::new),
    };
    Ok(if created {
        Created::new(format!("/orders/{}", sold.order_id), data).into_response()
    } else {
        Data { data }.into_response()
    })
}

/// Look up a product at the counter.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PosCatalogQuery {
    /// The open shift (its store's prices and stock).
    shift_id: Uuid,
    /// Barcode or SKU.
    barcode: String,
}

/// Look up a product by barcode at the counter.
///
/// Requires `orders.pos.sell` at the store of the open shift. Answers one unit priced like a sale
/// (with stock problems, if any).
#[utoipa::path(
    get,
    path = "/pos/catalog",
    extensions(("x-permissions" = json!(["orders.pos.sell"]))),
    operation_id = "lookup_pos_product",
    params(PosCatalogQuery),
    responses(
        (status = 200, description = "One unit of the product, priced.", body = Data<PosQuoteResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.pos.sell` at the store; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such shift; code `orders.shift_not_found`.", body = Problem),
        (status = 409, description = "The shift is not open; code `orders.shift_not_open`.", body = Problem),
        (status = 422, description = "No product with this code; code `orders.variant_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn catalog(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PosCatalogQuery>,
) -> Result<Data<PosQuoteResponse>, Error> {
    let quote = service.pos_lookup(&tenant, &principal, query.shift_id, &query.barcode).await?;
    Ok(Data { data: PosQuoteResponse::new(&quote) })
}
