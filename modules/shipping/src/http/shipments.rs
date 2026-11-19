//! Vận đơn: đọc (`shipping.shipments.read`), tạo tay / đặt / huỷ / tách / chia kiện / đổi trạng
//! thái (`shipping.shipments.manage`), nhãn của hãng.

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::addresses::PostalAddressRequest;
use crate::api::{Address, Direction, ShipmentItem, ShipmentRequest, ShipmentStatus};
use crate::entity::shipment;
use crate::errors;
use crate::permissions::{SHIPMENTS_MANAGE, SHIPMENTS_READ};
use crate::service::ShippingService;
use crate::shipments::{ParcelInput, ShipmentFilter, ShipmentView, status_of};

/// Filters of the shipment list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ShipmentQuery {
    order_id: Option<Uuid>,
    status: Option<ShipmentStatus>,
    org_id: Option<Uuid>,
    /// Tracking number of the carrier.
    tracking: Option<String>,
}

/// A shipment without its parcels, items and history.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingShipmentSummary)]
pub(crate) struct SummaryBody {
    id: Uuid,
    org_id: Uuid,
    origin_org_id: Uuid,
    order_id: Uuid,
    order_number: String,
    request_key: String,
    direction: Direction,
    method_id: Uuid,
    carrier_code: Option<String>,
    service_code: Option<String>,
    tracking_number: Option<String>,
    status: ShipmentStatus,
    status_at: DateTime<Utc>,
    recipient_name: String,
    recipient_phone: String,
    address: Address,
    declared_value: Money,
    cod_amount: Option<Money>,
    /// `none`, `pending`, `collected`, `settled` or `mismatch`.
    cod_status: String,
    quoted_fee: Option<Money>,
    actual_fee: Option<Money>,
    /// Last refusal of the carrier when booking (`shipping.carrier_error`…).
    last_error_code: Option<String>,
    created_at: DateTime<Utc>,
}

fn money(minor: i64, currency: &str) -> Money {
    Money::from_columns(minor, currency)
        .unwrap_or_else(|_| Money::new(0, autoxide_kernel::money::Currency::VND))
}

fn summary(row: shipment::Model) -> Result<SummaryBody, Error> {
    let address = crate::shipments::address_of_shipment(&row)?;
    Ok(SummaryBody {
        status: status_of(&row),
        direction: Direction::parse(&row.direction).unwrap_or(Direction::Outbound),
        declared_value: money(row.declared_value_minor, &row.currency),
        cod_amount: row.cod_minor.map(|minor| money(minor, &row.currency)),
        quoted_fee: row.quoted_fee_minor.map(|minor| money(minor, &row.currency)),
        actual_fee: row.actual_fee_minor.map(|minor| money(minor, &row.currency)),
        address,
        id: row.id,
        org_id: row.org_id,
        origin_org_id: row.origin_org_id,
        order_id: row.order_id,
        order_number: row.order_number,
        request_key: row.request_key,
        method_id: row.method_id,
        carrier_code: row.carrier_code,
        service_code: row.service_code,
        tracking_number: row.tracking_number,
        status_at: row.status_at,
        recipient_name: row.recipient_name,
        recipient_phone: row.recipient_phone,
        cod_status: row.cod_status,
        last_error_code: row.last_error_code,
        created_at: row.created_at,
    })
}

/// A parcel of a shipment.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingParcel)]
pub(crate) struct ParcelBody {
    id: Uuid,
    position: i32,
    weight_grams: i64,
    length_mm: Option<i64>,
    width_mm: Option<i64>,
    height_mm: Option<i64>,
    tracking_number: Option<String>,
}

/// An item of a shipment.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingShipmentItem)]
pub(crate) struct ItemBody {
    order_line_key: String,
    variant_id: Uuid,
    sku: String,
    name: String,
    quantity: i64,
    parcel_id: Option<Uuid>,
}

/// A step of the tracking history (only added, never changed).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingTrackingEvent)]
pub(crate) struct EventBody {
    /// `carrier`, `staff` or `system`.
    source: String,
    status: ShipmentStatus,
    /// The carrier's own wording.
    carrier_status: String,
    occurred_at: DateTime<Utc>,
    note: String,
}

/// A shipment with its parcels, items and tracking history.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingShipment)]
pub(crate) struct DetailBody {
    #[serde(flatten)]
    shipment: SummaryBody,
    parcels: Vec<ParcelBody>,
    items: Vec<ItemBody>,
    events: Vec<EventBody>,
}

fn detail(view: ShipmentView) -> Result<DetailBody, Error> {
    Ok(DetailBody {
        shipment: summary(view.shipment)?,
        parcels: view
            .parcels
            .into_iter()
            .map(|row| ParcelBody {
                id: row.id,
                position: row.position,
                weight_grams: row.weight_grams,
                length_mm: row.length_mm,
                width_mm: row.width_mm,
                height_mm: row.height_mm,
                tracking_number: row.tracking_number,
            })
            .collect(),
        items: view
            .items
            .into_iter()
            .map(|row| ItemBody {
                order_line_key: row.order_line_key,
                variant_id: row.variant_id,
                sku: row.sku,
                name: row.name,
                quantity: row.quantity,
                parcel_id: row.parcel_id,
            })
            .collect(),
        events: view
            .events
            .into_iter()
            .map(|row| EventBody {
                source: row.source,
                status: ShipmentStatus::parse(&row.status).unwrap_or(ShipmentStatus::Pending),
                carrier_status: row.carrier_status,
                occurred_at: row.occurred_at,
                note: row.note,
            })
            .collect(),
    })
}

/// List shipments.
///
/// Needs `shipping.shipments.read`; lists the shipments of the units it covers.
#[utoipa::path(
    get,
    path = "/shipments",
    extensions(("x-permissions" = json!(["shipping.shipments.read"]))),
    operation_id = "list_shipping_shipments",
    security(("bearer" = [])),
    params(ShipmentQuery, PageQuery),
    responses(
        (status = 200, description = "One page of shipments.", body = Page<SummaryBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.shipments.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_shipments(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ShipmentQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<SummaryBody>, Error> {
    principal.require(&SHIPMENTS_READ)?;
    let page = PageRequest::try_from(page)?;
    let filter = ShipmentFilter {
        order_id: query.order_id,
        status: query.status,
        org_id: query.org_id,
        tracking: query.tracking,
    };
    let page = service.list_shipments(&tenant, &principal, filter, &page).await?;
    let mut data = Vec::with_capacity(page.data.len());
    for row in page.data {
        data.push(summary(row)?);
    }
    Ok(Page { data, pagination: page.pagination })
}

/// Get a shipment with its parcels, items and tracking history.
#[utoipa::path(
    get,
    path = "/shipments/{id}",
    extensions(("x-permissions" = json!(["shipping.shipments.read"]))),
    operation_id = "get_shipping_shipment",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shipment.")),
    responses(
        (status = 200, description = "The shipment.", body = Data<DetailBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.shipments.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No shipment with this id in your scope; code `shipping.shipment_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_shipment(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<DetailBody>, Error> {
    principal.require(&SHIPMENTS_READ)?;
    Ok(Data { data: detail(service.get_shipment(&tenant, &principal, id).await?)? })
}

/// One line of a shipment request.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[schema(as = ShippingRequestItem)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestItemBody {
    order_line_key: String,
    variant_id: Uuid,
    #[serde(default)]
    sku: String,
    #[serde(default)]
    name: String,
    quantity: i64,
    weight_grams: Option<i64>,
    length_mm: Option<i64>,
    width_mm: Option<i64>,
    height_mm: Option<i64>,
}

/// A shipment created by staff (e.g. a counter sale delivered later).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = NewShippingShipment)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewShipmentRequest {
    order_id: Uuid,
    order_number: String,
    /// Retry-safe key of this delivery within the order.
    request_key: String,
    #[serde(default = "outbound")]
    direction: Direction,
    /// The selling store.
    org_id: Uuid,
    /// The warehouse shipping (default: the store).
    origin_org_id: Option<Uuid>,
    method_id: Uuid,
    buyer_id: Option<Uuid>,
    recipient_name: String,
    recipient_phone: String,
    address: PostalAddressRequest,
    items: Vec<RequestItemBody>,
    declared_value: Money,
    /// Cash the carrier collects on delivery.
    cod_amount: Option<Money>,
    #[serde(default)]
    note: String,
}

fn outbound() -> Direction {
    Direction::Outbound
}

/// Create a shipment by hand.
///
/// Needs `shipping.shipments.manage` at the store. Sending the same `order_id` and `request_key`
/// again returns the existing shipment (200); the same key with other content is refused.
#[utoipa::path(
    post,
    path = "/shipments",
    extensions(("x-permissions" = json!(["shipping.shipments.manage"]))),
    operation_id = "create_shipping_shipment",
    security(("bearer" = [])),
    request_body = NewShipmentRequest,
    responses(
        (status = 201, description = "Created (`pending`).", body = Data<DetailBody>),
        (status = 200, description = "The same request was already received; the existing shipment.", body = Data<DetailBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.shipments.manage` at the store; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 404, description = "No method with this id; code `shipping.method_not_found`.", body = Problem),
        (status = 409, description = "The request key was used with other content; code `shipping.request_conflict`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_shipment(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<NewShipmentRequest>,
) -> Result<axum::response::Response, Error> {
    principal.require(&SHIPMENTS_MANAGE)?;
    let address = service
        .check_address(&input.address.into(), "address.")
        .await
        .map_err(errors::invalid_address)?
        .address;
    let items = input
        .items
        .into_iter()
        .map(|item| {
            ShipmentItem::new(
                item.order_line_key,
                item.variant_id,
                item.sku,
                item.name,
                item.quantity,
            )
            .weight(item.weight_grams)
            .dimensions(item.length_mm, item.width_mm, item.height_mm)
        })
        .collect();
    let org = OrgId::from_uuid(input.org_id);
    let mut request = ShipmentRequest::new(
        input.order_id,
        input.order_number,
        input.request_key,
        org,
        input.origin_org_id.map_or(org, OrgId::from_uuid),
        input.method_id,
        (input.recipient_name, input.recipient_phone),
        address,
        items,
        input.declared_value,
    )
    .direction(input.direction)
    .note(input.note);
    if let Some(buyer) = input.buyer_id {
        request = request.buyer(buyer);
    }
    if let Some(cod) = input.cod_amount {
        request = request.cod(cod);
    }
    let (view, created) = service.create_shipment(&tenant, &principal, request).await?;
    let id = view.shipment.id;
    let body = detail(view)?;
    Ok(if created {
        Created::new(format!("/shipping/shipments/{id}"), body).into_response()
    } else {
        Data { data: body }.into_response()
    })
}

/// Book a shipment with its carrier.
///
/// Needs `shipping.shipments.manage`. Calls the carrier: on success the shipment gets its tracking
/// number and is `ready_to_ship`.
#[utoipa::path(
    post,
    path = "/shipments/{id}/book",
    extensions(("x-permissions" = json!(["shipping.shipments.manage"]))),
    operation_id = "book_shipping_shipment",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shipment.")),
    responses(
        (status = 200, description = "Booked.", body = Data<DetailBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.shipments.manage` at the store; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No shipment with this id in your scope; code `shipping.shipment_not_found`.", body = Problem),
        (status = 409, description = "Already booked (`shipping.already_booked`) or not bookable now (`shipping.invalid_transition`).", body = Problem),
        (status = 422, description = "The carrier refused (`shipping.booking_rejected`), or the method has no carrier (`shipping.no_carrier`).", body = Problem),
        (status = 503, description = "The carrier did not answer; the booking is retried. Code `shipping.carrier_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn book_shipment(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<DetailBody>, Error> {
    principal.require(&SHIPMENTS_MANAGE)?;
    Ok(Data { data: detail(service.book_by_staff(&tenant, &principal, id).await?)? })
}

/// Cancel a shipment.
///
/// Needs `shipping.shipments.manage`. A booked shipment is cancelled at the carrier too; once the
/// carrier picked it up it cannot be cancelled.
#[utoipa::path(
    post,
    path = "/shipments/{id}/cancel",
    extensions(("x-permissions" = json!(["shipping.shipments.manage"]))),
    operation_id = "cancel_shipping_shipment",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shipment.")),
    responses(
        (status = 200, description = "Cancelled.", body = Data<DetailBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.shipments.manage` at the store; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No shipment with this id in your scope; code `shipping.shipment_not_found`.", body = Problem),
        (status = 409, description = "The carrier already picked it up; code `shipping.cancel_too_late`.", body = Problem),
        (status = 503, description = "The carrier did not answer; code `shipping.carrier_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn cancel_shipment(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<DetailBody>, Error> {
    principal.require(&SHIPMENTS_MANAGE)?;
    Ok(Data { data: detail(service.cancel_by_staff(&tenant, &principal, id).await?)? })
}

/// Lines to move to a new shipment: order line key → quantity.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingSplitRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct SplitRequest {
    lines: BTreeMap<String, i64>,
}

/// Split a shipment (partial delivery).
///
/// Needs `shipping.shipments.manage`; only `pending` shipments. The lines given move to a new
/// shipment; cash on delivery stays with the original.
#[utoipa::path(
    post,
    path = "/shipments/{id}/split",
    extensions(("x-permissions" = json!(["shipping.shipments.manage"]))),
    operation_id = "split_shipping_shipment",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shipment.")),
    request_body = SplitRequest,
    responses(
        (status = 201, description = "The new shipment.", body = Data<DetailBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.shipments.manage` at the store; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No shipment with this id in your scope; code `shipping.shipment_not_found`.", body = Problem),
        (status = 409, description = "Already booked; code `shipping.already_booked`.", body = Problem),
        (status = 422, description = "A line or quantity is invalid, or every item would move; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn split_shipment(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SplitRequest>,
) -> Result<Created<DetailBody>, Error> {
    principal.require(&SHIPMENTS_MANAGE)?;
    let view = service.split(&tenant, &principal, id, input.lines).await?;
    let new_id = view.shipment.id;
    Ok(Created::new(format!("/shipping/shipments/{new_id}"), detail(view)?))
}

/// A parcel.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = ShippingParcelInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParcelRequest {
    weight_grams: i64,
    length_mm: Option<i64>,
    width_mm: Option<i64>,
    height_mm: Option<i64>,
    /// Order line keys packed in this parcel; lines not listed go in the first parcel.
    #[serde(default)]
    items: Vec<String>,
}

/// The parcels of a shipment.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingParcels)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParcelsRequest {
    parcels: Vec<ParcelRequest>,
}

/// Replace the parcels of a shipment.
///
/// Needs `shipping.shipments.manage`; only before booking.
#[utoipa::path(
    put,
    path = "/shipments/{id}/parcels",
    extensions(("x-permissions" = json!(["shipping.shipments.manage"]))),
    operation_id = "set_shipping_shipment_parcels",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shipment.")),
    request_body = ParcelsRequest,
    responses(
        (status = 200, description = "The shipment after the change.", body = Data<DetailBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.shipments.manage` at the store; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No shipment with this id in your scope; code `shipping.shipment_not_found`.", body = Problem),
        (status = 409, description = "Already booked; code `shipping.already_booked`.", body = Problem),
        (status = 422, description = "A parcel is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn set_parcels(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ParcelsRequest>,
) -> Result<Data<DetailBody>, Error> {
    principal.require(&SHIPMENTS_MANAGE)?;
    let parcels = input
        .parcels
        .into_iter()
        .map(|parcel| ParcelInput {
            weight_grams: parcel.weight_grams,
            length_mm: parcel.length_mm,
            width_mm: parcel.width_mm,
            height_mm: parcel.height_mm,
            items: parcel.items,
        })
        .collect();
    Ok(Data { data: detail(service.put_parcels(&tenant, &principal, id, parcels).await?)? })
}

/// A status set by staff.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingStatusRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct StatusRequest {
    status: ShipmentStatus,
    #[serde(default)]
    note: String,
}

/// Set the status of a shipment by hand.
///
/// Needs `shipping.shipments.manage`. For deliveries by the shop's own staff and store pickup
/// (`ready_for_pickup`, `picked_up_by_buyer`). Only moves allowed by the status table.
#[utoipa::path(
    post,
    path = "/shipments/{id}/status",
    extensions(("x-permissions" = json!(["shipping.shipments.manage"]))),
    operation_id = "set_shipping_shipment_status",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shipment.")),
    request_body = StatusRequest,
    responses(
        (status = 200, description = "The shipment after the change.", body = Data<DetailBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.shipments.manage` at the store; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No shipment with this id in your scope; code `shipping.shipment_not_found`.", body = Problem),
        (status = 409, description = "The status cannot go there from the current one; code `shipping.invalid_transition`.", body = Problem),
    )
)]
pub(crate) async fn set_status(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<StatusRequest>,
) -> Result<Data<DetailBody>, Error> {
    principal.require(&SHIPMENTS_MANAGE)?;
    let note: String = input.note.chars().take(1000).collect();
    Ok(Data {
        data: detail(service.staff_status(&tenant, &principal, id, input.status, note).await?)?,
    })
}

/// Download the carrier label of a shipment.
#[utoipa::path(
    get,
    path = "/shipments/{id}/label",
    extensions(("x-permissions" = json!(["shipping.shipments.read"]))),
    operation_id = "get_shipping_shipment_label",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shipment.")),
    responses(
        (status = 200, description = "The label file (usually PDF).", content_type = "application/octet-stream"),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.shipments.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No shipment with this id in your scope; code `shipping.shipment_not_found`.", body = Problem),
        (status = 409, description = "Not booked with a carrier yet; code `shipping.not_booked`.", body = Problem),
        (status = 503, description = "The carrier did not answer; code `shipping.carrier_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn get_label(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<axum::response::Response, Error> {
    principal.require(&SHIPMENTS_READ)?;
    let label = service.label(&tenant, &principal, id).await?;
    Ok((StatusCode::OK, [(header::CONTENT_TYPE, label.content_type)], label.bytes).into_response())
}
