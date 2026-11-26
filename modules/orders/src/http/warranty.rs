//! Endpoint bảo hành (đặc tả 45 mục 14): ghi serial của dòng đơn, tra bảo hành, phiếu bảo hành.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::entity::{warranty, warranty_claim};
use crate::service::OrdersService;

/// Serial numbers of an order line.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersSerialsRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct SerialsRequest {
    /// Serial numbers to add (1 to 100 characters each, no repeats); at most the line quantity in
    /// all.
    #[schema(example = json!(["SN-0001"]))]
    serials: Vec<String>,
}

/// The serial numbers recorded for an order line.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersSerials)]
pub(crate) struct SerialsResponse {
    serials: Vec<String>,
}

/// Record serial numbers of an order line.
///
/// Requires `orders.manage` at the selling node. Usually done while preparing the order; each
/// serial gets its own warranty when the order is delivered (at once, if it already is). Adding
/// a serial that is already recorded changes nothing.
#[utoipa::path(
    post,
    path = "/{id}/lines/{line_id}/serials",
    extensions(("x-permissions" = json!(["orders.manage"]))),
    operation_id = "record_line_serials",
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("line_id" = Uuid, Path, description = "Id of the order line."),
    ),
    request_body = SerialsRequest,
    responses(
        (status = 200, description = "Every serial recorded for the line.", body = Data<SerialsResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.manage` at the node; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order or line; code `orders.order_not_found`.", body = Problem),
        (status = 422, description = "Empty, too long or repeated serials, or more than the quantity (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn record_serials(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, line_id)): Path<(Uuid, Uuid)>,
    ValidJson(input): ValidJson<SerialsRequest>,
) -> Result<Data<SerialsResponse>, Error> {
    let serials = service.record_serials(&tenant, &principal, id, line_id, input.serials).await?;
    Ok(Data { data: SerialsResponse { serials } })
}

/// A warranty of an order line (or of one serial number).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersWarranty)]
pub(crate) struct WarrantyResponse {
    id: Uuid,
    org_id: Uuid,
    order_id: Uuid,
    order_line_id: Uuid,
    product_id: Uuid,
    variant_id: Uuid,
    serial: Option<String>,
    /// When the order was delivered.
    starts_at: Timestamp,
    ends_at: Timestamp,
    /// `active` or `expired`.
    status: String,
}

impl WarrantyResponse {
    pub(crate) fn new(row: warranty::Model) -> Self {
        WarrantyResponse {
            id: row.id,
            org_id: row.org_id,
            order_id: row.order_id,
            order_line_id: row.order_line_id,
            product_id: row.product_id,
            variant_id: row.variant_id,
            serial: row.serial,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            status: row.status,
        }
    }
}

/// Find warranties by serial number or buyer phone.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct WarrantyQuery {
    /// Serial number printed on the product.
    serial: Option<String>,
    /// Phone number the buyer ordered with.
    phone: Option<String>,
}

/// Look up warranties.
///
/// Requires `orders.warranty.manage`; only warranties of nodes in your scope. Give `serial`,
/// `phone` or both (either matches). Newest first unless `order=asc`; paged with `limit` /
/// `cursor`.
#[utoipa::path(
    get,
    path = "/warranties",
    extensions(("x-permissions" = json!(["orders.warranty.manage"]))),
    operation_id = "find_warranties",
    params(WarrantyQuery, PageQuery),
    responses(
        (status = 200, description = "One page of the matching warranties.", body = Page<WarrantyResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.warranty.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Neither `serial` nor `phone`, not a phone number, or a paging parameter is invalid (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn find_warranties(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<WarrantyQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<WarrantyResponse>, Error> {
    principal.require(&crate::permissions::WARRANTY_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    let rows = service
        .find_warranties(
            &tenant,
            &principal,
            query.serial.as_deref(),
            query.phone.as_deref(),
            &page,
        )
        .await?;
    Ok(rows.map(WarrantyResponse::new))
}

/// Your warranties.
///
/// Signed in: warranties of products you ordered with your account. Newest first unless
/// `order=asc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/me/warranties",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_warranties",
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your warranties.", body = Page<WarrantyResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn my_warranties(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<WarrantyResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    let rows = service.my_warranties(&tenant, principal.user_id(), &page).await?;
    Ok(rows.map(WarrantyResponse::new))
}

/// A warranty claim.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersWarrantyClaim)]
pub(crate) struct ClaimResponse {
    id: Uuid,
    warranty_id: Uuid,
    /// The node that received the product.
    org_id: Uuid,
    /// Claim number shown to the buyer (`BH<year>-<sequence>`).
    number: String,
    /// `received`, `inspecting`, `repairing`, `replacing`, `ready`, `returned` or `rejected`.
    status: String,
    issue: String,
    resolution: Option<String>,
    /// When the seller must have handled the claim.
    due_at: Timestamp,
    created_by: Uuid,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl ClaimResponse {
    pub(crate) fn new(row: warranty_claim::Model) -> Self {
        ClaimResponse {
            id: row.id,
            warranty_id: row.warranty_id,
            org_id: row.org_id,
            number: row.number,
            status: row.status,
            issue: row.issue,
            resolution: row.resolution,
            due_at: row.due_at,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A product received for warranty.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersWarrantyClaimRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimRequest {
    warranty_id: Uuid,
    /// The node receiving the product.
    org_id: Uuid,
    /// What is wrong, as the buyer tells it (1 to 5000 characters).
    #[schema(example = "The screen stays black.")]
    issue: String,
}

/// Receive a product for warranty.
///
/// Requires `orders.warranty.manage` at the receiving node. The warranty must still be active.
/// The claim gets a number and a due date (`orders.warranty_claim_days` of the node).
#[utoipa::path(
    post,
    path = "/warranty-claims",
    extensions(("x-permissions" = json!(["orders.warranty.manage"]))),
    operation_id = "create_warranty_claim",
    request_body = ClaimRequest,
    responses(
        (status = 201, description = "The claim.", body = Data<ClaimResponse>, headers(("Location" = String, description = "Path of the claim."))),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.warranty.manage` at the node; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such warranty; code `orders.warranty_not_found`.", body = Problem),
        (status = 409, description = "The warranty has ended; code `orders.warranty_expired`.", body = Problem),
        (status = 422, description = "The issue is empty or too long (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn create_claim(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ClaimRequest>,
) -> Result<Created<ClaimResponse>, Error> {
    let org = OrgId::from_uuid(input.org_id);
    let row = service.open_claim(&tenant, &principal, input.warranty_id, org, &input.issue).await?;
    let path = format!("/orders/warranty-claims/{}", row.id);
    Ok(Created::new(path, ClaimResponse::new(row)))
}

/// A change of a warranty claim's status.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersWarrantyClaimTransition)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimTransitionRequest {
    /// `inspecting`, `repairing`, `replacing`, `ready`, `returned` or `rejected`.
    #[schema(example = "inspecting")]
    to: String,
    /// What was done (or why it was rejected); kept when omitted.
    #[validate(length(min = 1, max = 5000, message = "must be 1 to 5000 characters"))]
    resolution: Option<String>,
}

/// Move a warranty claim on.
///
/// Requires `orders.warranty.manage` at the claim's node. Allowed: `received` → `inspecting` or
/// `rejected`; `inspecting` → `repairing`, `replacing` or `rejected`; `repairing` / `replacing` →
/// `ready`; `ready` → `returned`. Asking for the current status again changes nothing.
#[utoipa::path(
    post,
    path = "/warranty-claims/{id}/transitions",
    extensions(("x-permissions" = json!(["orders.warranty.manage"]))),
    operation_id = "transition_warranty_claim",
    params(("id" = Uuid, Path, description = "Id of the claim.")),
    request_body = ClaimTransitionRequest,
    responses(
        (status = 200, description = "The claim.", body = Data<ClaimResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.warranty.manage` at the node; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such claim; code `orders.warranty_claim_not_found`.", body = Problem),
        (status = 409, description = "Not allowed from the current status; code `orders.warranty_claim_transition`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn transition_claim(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ClaimTransitionRequest>,
) -> Result<Data<ClaimResponse>, Error> {
    let row =
        service.transition_claim(&tenant, &principal, id, &input.to, input.resolution).await?;
    Ok(Data { data: ClaimResponse::new(row) })
}
