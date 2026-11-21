//! Endpoint chỗ giữ hàng (xem, gỡ kẹt bằng tay), kho phục vụ cửa hàng và cài đặt chọn kho.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{Reservation, ReservationStatus};
use crate::permissions::{ADJUST, RESERVATIONS_READ, SETTINGS_MANAGE};
use crate::service::InventoryService;
use crate::settings::{SourceInput, StoreSettings};

pub(crate) fn routes(service: Arc<InventoryService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_reservations))
        .routes(routes!(release_reservation))
        .routes(routes!(get_sources, put_sources))
        .routes(routes!(get_settings, put_settings))
        .with_state(service)
}

/// Status of a stock reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = InventoryReservationStatus)]
pub(crate) enum StatusDto {
    Held,
    Confirmed,
    Fulfilled,
    Released,
    Expired,
}

impl From<StatusDto> for ReservationStatus {
    fn from(status: StatusDto) -> Self {
        match status {
            StatusDto::Held => ReservationStatus::Held,
            StatusDto::Confirmed => ReservationStatus::Confirmed,
            StatusDto::Fulfilled => ReservationStatus::Fulfilled,
            StatusDto::Released => ReservationStatus::Released,
            StatusDto::Expired => ReservationStatus::Expired,
        }
    }
}

/// Part of an order line reserved in one warehouse.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryReservedLine)]
pub(crate) struct ReservedLineResponse {
    /// The order's line key.
    pub line_key: String,
    /// The variant taken from stock (a component when the line is a bundle).
    pub variant_id: Uuid,
    /// The bundle variant of the order line, when it is a bundle.
    pub bundle_variant_id: Option<Uuid>,
    pub warehouse_org_id: Uuid,
    pub quantity: i64,
    pub fulfilled_quantity: i64,
    pub released_quantity: i64,
    /// Reserved beyond the available stock (the product may be sold out of stock).
    pub backordered: bool,
    /// Pre-order line.
    pub preordered: bool,
}

/// Stock held for one order.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryReservation)]
pub(crate) struct ReservationResponse {
    pub id: Uuid,
    pub order_id: Uuid,
    /// The store or channel that sold.
    pub selling_org_id: Uuid,
    /// `held` (until `expires_at`), `confirmed`, `fulfilled`, `released` or `expired`.
    pub status: String,
    pub expires_at: Option<Timestamp>,
    pub lines: Vec<ReservedLineResponse>,
}

impl From<Reservation> for ReservationResponse {
    fn from(reservation: Reservation) -> Self {
        ReservationResponse {
            id: reservation.id,
            order_id: reservation.order_id,
            selling_org_id: reservation.selling_org_id.as_uuid(),
            status: reservation.status.as_str().to_owned(),
            expires_at: reservation.expires_at,
            lines: reservation
                .lines
                .into_iter()
                .map(|line| ReservedLineResponse {
                    line_key: line.line_key,
                    variant_id: line.variant_id,
                    bundle_variant_id: line.bundle_variant_id,
                    warehouse_org_id: line.warehouse_org_id.as_uuid(),
                    quantity: line.quantity,
                    fulfilled_quantity: line.fulfilled_quantity,
                    released_quantity: line.released_quantity,
                    backordered: line.backordered,
                    preordered: line.preordered,
                })
                .collect(),
        }
    }
}

/// Filters of the reservation list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ReservationsQuery {
    /// Only the reservation of this order.
    pub order_id: Option<Uuid>,
    /// Only reservations in this status.
    #[param(inline)]
    pub status: Option<StatusDto>,
}

/// List stock reservations of orders sold by units in your scope.
///
/// Requires `inventory.reservations.read`; its scope applies to the selling unit. Newest first
/// unless `order=asc`.
#[utoipa::path(
    get,
    path = "/reservations",
    extensions(("x-permissions" = json!(["inventory.reservations.read"]))),
    operation_id = "list_inventory_reservations",
    security(("bearer" = [])),
    params(ReservationsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of reservations.", body = Page<ReservationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.reservations.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_reservations(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ReservationsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReservationResponse>, Error> {
    principal.require(&RESERVATIONS_READ)?;
    let page = PageRequest::try_from(page)?;
    let found = service
        .list_reservations(&tenant, &principal, query.order_id, query.status.map(Into::into), &page)
        .await?;
    Ok(found.map(ReservationResponse::from))
}

/// Release a stuck reservation by hand.
///
/// Requires `inventory.adjust` covering the selling unit. Units not shipped yet go back to the
/// available stock; the release is recorded (`inventory.reservation_released` with your user
/// id, reason `manual`). Releasing again returns the reservation as it is.
#[utoipa::path(
    post,
    path = "/reservations/{id}/release",
    extensions(("x-permissions" = json!(["inventory.adjust"]))),
    operation_id = "release_inventory_reservation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the reservation.")),
    responses(
        (status = 200, description = "The reservation after the release.", body = Data<ReservationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.adjust` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No reservation with this id in your scope; code `inventory.reservation_not_found`.", body = Problem),
    )
)]
async fn release_reservation(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ReservationResponse>, Error> {
    principal.require(&ADJUST)?;
    let reservation = service.release_by_hand(&tenant, &principal, id).await?;
    Ok(Data { data: reservation.into() })
}

/// One warehouse serving a store or channel.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventorySource)]
pub(crate) struct SourceDto {
    /// The warehouse or store whose stock is sold.
    pub warehouse_org_id: Uuid,
    /// Lower is tried first.
    pub priority: i32,
    /// `false` keeps the row but stops using the warehouse.
    pub active: bool,
}

/// The warehouses serving a store, replacing the current list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventorySourcesRequest)]
pub(crate) struct SourcesRequest {
    /// Up to 100 warehouses; empty: the store sells only its own stock.
    pub sources: Vec<SourceDto>,
}

/// Warehouses serving a store.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventorySources)]
pub(crate) struct SourcesResponse {
    pub sources: Vec<SourceDto>,
}

/// List the warehouses serving a store or channel, in the order they are tried.
///
/// Requires `inventory.settings.manage` covering the selling unit. Without rows the store sells
/// its own stock (when it is a warehouse or store).
#[utoipa::path(
    get,
    path = "/sources/{selling_org_id}",
    extensions(("x-permissions" = json!(["inventory.settings.manage"]))),
    operation_id = "get_inventory_sources",
    security(("bearer" = [])),
    params(("selling_org_id" = Uuid, Path, description = "The selling unit.")),
    responses(
        (status = 200, description = "The warehouses.", body = Data<SourcesResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.settings.manage` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
    )
)]
async fn get_sources(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(selling_org_id): Path<Uuid>,
) -> Result<Data<SourcesResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let sources = service.sources(&tenant, &principal, selling_org_id).await?;
    Ok(Data { data: present_sources(sources) })
}

fn present_sources(sources: Vec<SourceInput>) -> SourcesResponse {
    SourcesResponse {
        sources: sources
            .into_iter()
            .map(|source| SourceDto {
                warehouse_org_id: source.warehouse_org_id,
                priority: source.priority,
                active: source.active,
            })
            .collect(),
    }
}

/// Replace the warehouses serving a store or channel.
///
/// Requires `inventory.settings.manage` covering the selling unit. Every warehouse must be a
/// warehouse or store.
#[utoipa::path(
    put,
    path = "/sources/{selling_org_id}",
    extensions(("x-permissions" = json!(["inventory.settings.manage"]))),
    operation_id = "put_inventory_sources",
    security(("bearer" = [])),
    params(("selling_org_id" = Uuid, Path, description = "The selling unit.")),
    request_body = SourcesRequest,
    responses(
        (status = 200, description = "The warehouses now serving the unit.", body = Data<SourcesResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.settings.manage` (`permission_denied`), or the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`).", body = Problem),
        (status = 422, description = "A unit holds no stock (`inventory.not_a_warehouse`), a warehouse is listed twice, or the list is too long; see `errors`.", body = Problem),
    )
)]
async fn put_sources(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(selling_org_id): Path<Uuid>,
    ValidJson(input): ValidJson<SourcesRequest>,
) -> Result<Data<SourcesResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let sources = input
        .sources
        .into_iter()
        .map(|source| SourceInput {
            warehouse_org_id: source.warehouse_org_id,
            priority: source.priority,
            active: source.active,
        })
        .collect();
    let saved = service.set_sources(&tenant, &principal, selling_org_id, sources).await?;
    Ok(Data { data: present_sources(saved) })
}

/// How a store picks warehouses.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryStoreSettings)]
pub(crate) struct SettingsDto {
    /// Try warehouses in the buyer's province first (province from the units' public profile).
    pub prefer_nearby: bool,
    /// Split an order over several warehouses when no single one has every line. Default
    /// `false`: one order, one warehouse.
    pub allow_split: bool,
}

impl From<StoreSettings> for SettingsDto {
    fn from(settings: StoreSettings) -> Self {
        SettingsDto { prefer_nearby: settings.prefer_nearby, allow_split: settings.allow_split }
    }
}

/// Get how a store picks warehouses.
///
/// Requires `inventory.settings.manage` covering the unit.
#[utoipa::path(
    get,
    path = "/settings/{org_id}",
    extensions(("x-permissions" = json!(["inventory.settings.manage"]))),
    operation_id = "get_inventory_settings",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "The selling unit.")),
    responses(
        (status = 200, description = "The settings (defaults when never set).", body = Data<SettingsDto>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.settings.manage` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
    )
)]
async fn get_settings(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
) -> Result<Data<SettingsDto>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.store_settings(&tenant, &principal, org_id).await?.into() })
}

/// Change how a store picks warehouses.
///
/// Requires `inventory.settings.manage` covering the unit.
#[utoipa::path(
    put,
    path = "/settings/{org_id}",
    extensions(("x-permissions" = json!(["inventory.settings.manage"]))),
    operation_id = "put_inventory_settings",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "The selling unit.")),
    request_body = SettingsDto,
    responses(
        (status = 200, description = "The settings.", body = Data<SettingsDto>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.settings.manage` (`permission_denied`), or the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`).", body = Problem),
    )
)]
async fn put_settings(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
    ValidJson(input): ValidJson<SettingsDto>,
) -> Result<Data<SettingsDto>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let settings =
        StoreSettings { prefer_nearby: input.prefer_nearby, allow_split: input.allow_split };
    Ok(Data {
        data: service.set_store_settings(&tenant, &principal, org_id, settings).await?.into(),
    })
}
