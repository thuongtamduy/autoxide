//! Endpoint phiếu chuyển kho và phiếu kiểm kê, dưới `/api/v1/inventory`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::counts::{Count, CountedLine};
use crate::permissions::{ADJUST, COUNT, READ, TRANSFER};
use crate::service::InventoryService;
use crate::transfers::{NewTransfer, ReceiveLine, Transfer, TransferLineInput};

pub(crate) fn routes(service: Arc<InventoryService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_transfers, create_transfer))
        .routes(routes!(get_transfer))
        .routes(routes!(ship_transfer))
        .routes(routes!(receive_transfer))
        .routes(routes!(close_transfer))
        .routes(routes!(cancel_transfer))
        .routes(routes!(list_counts, open_count))
        .routes(routes!(get_count))
        .routes(routes!(put_count_lines))
        .routes(routes!(import_count))
        .routes(routes!(apply_count))
        .routes(routes!(cancel_count))
        .with_state(service)
}

/// One line of a stock transfer.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryTransferLine)]
pub(crate) struct TransferLineResponse {
    pub variant_id: Uuid,
    pub quantity: i64,
    pub received_quantity: i64,
    /// Average cost at the source when shipped; the destination's average cost uses it.
    pub unit_cost: Option<autoxide_kernel::money::Money>,
}

/// A stock transfer between two warehouses.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryTransfer)]
pub(crate) struct TransferResponse {
    pub id: Uuid,
    /// Document number, e.g. `CK-2026-000001`.
    pub number: String,
    pub from_org_id: Uuid,
    pub to_org_id: Uuid,
    /// `draft`, `in_transit`, `partially_received`, `received`, `closed` or `cancelled`.
    pub status: String,
    pub shipped_at: Option<Timestamp>,
    pub reference: Option<String>,
    pub note: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub lines: Vec<TransferLineResponse>,
}

impl From<Transfer> for TransferResponse {
    fn from(transfer: Transfer) -> Self {
        let row = transfer.row;
        TransferResponse {
            id: row.id,
            number: row.number,
            from_org_id: row.from_org_id,
            to_org_id: row.to_org_id,
            status: row.status,
            shipped_at: row.shipped_at,
            reference: row.reference,
            note: row.note,
            created_at: row.created_at,
            updated_at: row.updated_at,
            lines: transfer
                .lines
                .into_iter()
                .map(|line| TransferLineResponse {
                    variant_id: line.variant_id,
                    quantity: line.quantity,
                    received_quantity: line.received_quantity,
                    unit_cost: match (line.unit_cost_minor, line.unit_cost_currency.as_deref()) {
                        (Some(minor), Some(currency)) => {
                            autoxide_kernel::money::Money::from_columns(minor, currency).ok()
                        }
                        _ => None,
                    },
                })
                .collect(),
        }
    }
}

/// A variant and a quantity.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryQuantityLine)]
pub(crate) struct QuantityLine {
    pub variant_id: Uuid,
    /// 1 to 1 000 000.
    pub quantity: i64,
}

/// One line of a transfer to create.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryTransferLineRequest)]
pub(crate) struct TransferLineRequest {
    pub variant_id: Uuid,
    /// 1 to 1 000 000.
    pub quantity: i64,
    /// Exactly `quantity` serial numbers when the variant tracks serials.
    #[serde(default)]
    pub serials: Vec<String>,
}

/// A stock transfer to create (as a draft).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryTransferRequest)]
pub(crate) struct TransferRequest {
    /// The warehouse the goods leave.
    pub from_org_id: Uuid,
    /// The warehouse the goods go to.
    pub to_org_id: Uuid,
    /// Your own reference, up to 100 characters.
    pub reference: Option<String>,
    /// Up to 500 characters.
    pub note: Option<String>,
    /// 1 to 500 lines, one per variant.
    pub lines: Vec<TransferLineRequest>,
}

/// Units received now; no lines receives everything still in transit.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryReceiveRequest)]
pub(crate) struct ReceiveRequest {
    #[serde(default)]
    pub lines: Vec<QuantityLine>,
}

/// Filters of the transfer list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct TransfersQuery {
    /// `draft`, `in_transit`, `partially_received`, `received`, `closed` or `cancelled`.
    pub status: Option<String>,
    /// Transfers from or to this warehouse.
    pub org_id: Option<Uuid>,
}

/// List stock transfers from or to warehouses in your scope.
///
/// Requires `inventory.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/transfers",
    extensions(("x-permissions" = json!(["inventory.read"]))),
    operation_id = "list_inventory_transfers",
    security(("bearer" = [])),
    params(TransfersQuery, PageQuery),
    responses(
        (status = 200, description = "One page of transfers.", body = Page<TransferResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_transfers(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<TransfersQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<TransferResponse>, Error> {
    principal.require(&READ)?;
    let page = PageRequest::try_from(page)?;
    let found = service
        .list_transfers(&tenant, &principal, query.status.as_deref(), query.org_id, &page)
        .await?;
    Ok(found.map(TransferResponse::from))
}

/// Create a stock transfer as a draft.
///
/// Requires `inventory.transfer` covering the source warehouse and the plan feature
/// `inventory.transfers`. Nothing moves until `POST /transfers/{id}/ship`. Retry safely with an
/// `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/transfers",
    extensions(("x-permissions" = json!(["inventory.transfer"]))),
    operation_id = "create_inventory_transfer",
    security(("bearer" = [])),
    request_body = TransferRequest,
    responses(
        (status = 201, description = "The draft transfer.", body = Data<TransferResponse>, headers(("Location" = String, description = "Path of the transfer."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.transfer` (`permission_denied`), the source is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`), or the plan lacks transfers (`feature_not_enabled`).", body = Problem),
        (status = 404, description = "A variant does not exist; code `inventory.variant_not_found`.", body = Problem),
        (status = 422, description = "A unit holds no stock (`inventory.not_a_warehouse`), a variant tracks no stock (`inventory.not_tracked`), or a value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_transfer(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<TransferRequest>,
) -> Result<Created<TransferResponse>, Error> {
    principal.require(&TRANSFER)?;
    let input = NewTransfer {
        from_org_id: input.from_org_id,
        to_org_id: input.to_org_id,
        reference: input.reference,
        note: input.note,
        lines: input
            .lines
            .into_iter()
            .map(|line| TransferLineInput {
                variant_id: line.variant_id,
                quantity: line.quantity,
                serials: line.serials,
            })
            .collect(),
    };
    let transfer =
        TransferResponse::from(service.create_transfer(&tenant, &principal, input).await?);
    Ok(Created::new(format!("/inventory/transfers/{}", transfer.id), transfer))
}

/// Get a stock transfer.
///
/// Requires `inventory.read` covering the source or the destination.
#[utoipa::path(
    get,
    path = "/transfers/{id}",
    extensions(("x-permissions" = json!(["inventory.read"]))),
    operation_id = "get_inventory_transfer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the transfer.")),
    responses(
        (status = 200, description = "The transfer.", body = Data<TransferResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No transfer with this id in your scope; code `inventory.transfer_not_found`.", body = Problem),
    )
)]
async fn get_transfer(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TransferResponse>, Error> {
    principal.require(&READ)?;
    Ok(Data { data: service.get_transfer(&tenant, &principal, id).await?.into() })
}

/// Ship a draft transfer: the goods leave the source and are in transit.
///
/// Requires `inventory.transfer` covering the source. Takes only stock not reserved for orders;
/// adds the units to `incoming` at the destination.
#[utoipa::path(
    post,
    path = "/transfers/{id}/ship",
    extensions(("x-permissions" = json!(["inventory.transfer"]))),
    operation_id = "ship_inventory_transfer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the transfer.")),
    responses(
        (status = 200, description = "The transfer, now `in_transit`.", body = Data<TransferResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.transfer` at the source (`permission_denied`, `org_out_of_scope`) or it is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No transfer with this id in your scope; code `inventory.transfer_not_found`.", body = Problem),
        (status = 409, description = "Not a draft (`inventory.invalid_transition`) or not enough available stock (`inventory.insufficient_stock`).", body = Problem),
    )
)]
async fn ship_transfer(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TransferResponse>, Error> {
    principal.require(&TRANSFER)?;
    Ok(Data { data: service.ship_transfer(&tenant, &principal, id).await?.into() })
}

/// Receive goods of a transfer in transit, all or part.
///
/// Requires `inventory.transfer` covering the destination. Without lines, everything still in
/// transit is received. The destination's average cost uses the cost at the source.
#[utoipa::path(
    post,
    path = "/transfers/{id}/receive",
    extensions(("x-permissions" = json!(["inventory.transfer"]))),
    operation_id = "receive_inventory_transfer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the transfer.")),
    request_body = ReceiveRequest,
    responses(
        (status = 200, description = "The transfer, `partially_received` or `received`.", body = Data<TransferResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.transfer` at the destination (`permission_denied`, `org_out_of_scope`) or it is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No transfer with this id in your scope; code `inventory.transfer_not_found`.", body = Problem),
        (status = 409, description = "Not in transit (`inventory.invalid_transition`) or more than in transit (`inventory.over_receipt`).", body = Problem),
        (status = 422, description = "A quantity is invalid; see `errors`.", body = Problem),
    )
)]
async fn receive_transfer(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReceiveRequest>,
) -> Result<Data<TransferResponse>, Error> {
    principal.require(&TRANSFER)?;
    let lines = input
        .lines
        .into_iter()
        .map(|line| ReceiveLine { variant_id: line.variant_id, quantity: line.quantity })
        .collect();
    Ok(Data { data: service.receive_transfer(&tenant, &principal, id, lines).await?.into() })
}

/// Close a transfer in transit: what has not arrived is written off from `incoming`.
///
/// Requires `inventory.transfer` covering the destination. Record a loss with an adjustment if
/// needed.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post,
    path = "/transfers/{id}/close",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["inventory.transfer"]))),
    operation_id = "close_inventory_transfer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the transfer.")),
    responses(
        (status = 200, description = "The transfer, now `closed`.", body = Data<TransferResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.transfer` at the destination (`permission_denied`, `org_out_of_scope`) or it is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No transfer with this id in your scope; code `inventory.transfer_not_found`.", body = Problem),
        (status = 409, description = "Not in transit; code `inventory.invalid_transition`.", body = Problem),
    )
)]
async fn close_transfer(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TransferResponse>, Error> {
    principal.require(&TRANSFER)?;
    Ok(Data { data: service.finish_transfer(&tenant, &principal, id, false).await?.into() })
}

/// Cancel a draft transfer.
///
/// Requires `inventory.transfer` covering the source.
#[utoipa::path(
    post,
    path = "/transfers/{id}/cancel",
    extensions(("x-permissions" = json!(["inventory.transfer"]))),
    operation_id = "cancel_inventory_transfer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the transfer.")),
    responses(
        (status = 200, description = "The transfer, now `cancelled`.", body = Data<TransferResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.transfer` at the source (`permission_denied`, `org_out_of_scope`) or it is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No transfer with this id in your scope; code `inventory.transfer_not_found`.", body = Problem),
        (status = 409, description = "Not a draft; code `inventory.invalid_transition`.", body = Problem),
    )
)]
async fn cancel_transfer(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TransferResponse>, Error> {
    principal.require(&TRANSFER)?;
    Ok(Data { data: service.finish_transfer(&tenant, &principal, id, true).await?.into() })
}

/// One counted line of a stock count.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryCountLine)]
pub(crate) struct CountLineResponse {
    pub variant_id: Uuid,
    pub counted: i64,
    /// On hand when this line was counted; the difference applied is `counted − expected_at_count`.
    pub expected_at_count: i64,
    pub counted_at: Timestamp,
    pub reason_code: Option<String>,
}

/// A stock count of one warehouse.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryCount)]
pub(crate) struct CountResponse {
    pub id: Uuid,
    /// Document number, e.g. `KK-2026-000001`.
    pub number: String,
    pub org_id: Uuid,
    /// `open`, `applied` or `cancelled`.
    pub status: String,
    pub note: String,
    pub applied_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub lines: Vec<CountLineResponse>,
}

impl From<Count> for CountResponse {
    fn from(count: Count) -> Self {
        let row = count.row;
        CountResponse {
            id: row.id,
            number: row.number,
            org_id: row.org_id,
            status: row.status,
            note: row.note,
            applied_at: row.applied_at,
            created_at: row.created_at,
            lines: count
                .lines
                .into_iter()
                .map(|line| CountLineResponse {
                    variant_id: line.variant_id,
                    counted: line.counted,
                    expected_at_count: line.expected_at_count,
                    counted_at: line.counted_at,
                    reason_code: line.reason_code,
                })
                .collect(),
        }
    }
}

/// A stock count to open.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryCountRequest)]
pub(crate) struct CountRequest {
    /// The warehouse or store counted.
    pub org_id: Uuid,
    /// Up to 500 characters.
    pub note: Option<String>,
}

/// One counted quantity.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryCountedLine)]
pub(crate) struct CountedLineRequest {
    pub variant_id: Uuid,
    /// Units counted, 0 to 1 000 000.
    pub counted: i64,
    /// Reason for a difference, from the list `inventory.adjustment`; default `count_variance`.
    pub reason_code: Option<String>,
}

/// Counted quantities to record (replacing earlier counts of the same variants).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryCountLinesRequest)]
pub(crate) struct CountLinesRequest {
    /// 1 to 5000 lines.
    pub lines: Vec<CountedLineRequest>,
}

/// Counted quantities as CSV text.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryCountImportRequest)]
pub(crate) struct CountImportRequest {
    /// CSV with a header row and the columns `sku` (or barcode), `counted` and optionally
    /// `reason_code`; up to 5000 rows.
    pub csv: String,
}

/// Filters of the stock count list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct CountsQuery {
    /// Only counts of this warehouse or store.
    pub org_id: Option<Uuid>,
    /// `open`, `applied` or `cancelled`.
    pub status: Option<String>,
}

/// List stock counts in your scope.
///
/// Requires `inventory.count`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/counts",
    extensions(("x-permissions" = json!(["inventory.count"]))),
    operation_id = "list_inventory_counts",
    security(("bearer" = [])),
    params(CountsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of stock counts.", body = Page<CountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.count`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_counts(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<CountsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<CountResponse>, Error> {
    principal.require(&COUNT)?;
    let page = PageRequest::try_from(page)?;
    let found = service
        .list_counts(&tenant, &principal, query.org_id, query.status.as_deref(), &page)
        .await?;
    Ok(found.map(CountResponse::from))
}

/// Open a stock count in one warehouse.
///
/// Requires `inventory.count` covering the warehouse. Selling goes on while counting: each line
/// keeps the on-hand quantity at the moment it was counted.
#[utoipa::path(
    post,
    path = "/counts",
    extensions(("x-permissions" = json!(["inventory.count"]))),
    operation_id = "open_inventory_count",
    security(("bearer" = [])),
    request_body = CountRequest,
    responses(
        (status = 201, description = "The open stock count.", body = Data<CountResponse>, headers(("Location" = String, description = "Path of the stock count."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.count` (`permission_denied`), or the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`).", body = Problem),
        (status = 422, description = "The unit holds no stock (`inventory.not_a_warehouse`) or a value is invalid; see `errors`.", body = Problem),
    )
)]
async fn open_count(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CountRequest>,
) -> Result<Created<CountResponse>, Error> {
    principal.require(&COUNT)?;
    let count = CountResponse::from(
        service.open_count(&tenant, &principal, input.org_id, input.note).await?,
    );
    Ok(Created::new(format!("/inventory/counts/{}", count.id), count))
}

/// Get a stock count with its lines.
///
/// Requires `inventory.read` or `inventory.count` covering the warehouse.
#[utoipa::path(
    get,
    path = "/counts/{id}",
    extensions(("x-permissions" = json!(["inventory.count", "inventory.read"]))),
    operation_id = "get_inventory_count",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the stock count.")),
    responses(
        (status = 200, description = "The stock count.", body = Data<CountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.read` and `inventory.count`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No stock count with this id in your scope; code `inventory.count_not_found`.", body = Problem),
    )
)]
async fn get_count(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CountResponse>, Error> {
    if !principal.has(&READ) {
        principal.require(&COUNT)?;
    }
    Ok(Data { data: service.get_count(&tenant, &principal, id).await?.into() })
}

/// Record counted quantities.
///
/// Requires `inventory.count` covering the warehouse. Counting a variant again replaces its line
/// (and its on-hand snapshot).
#[utoipa::path(
    put,
    path = "/counts/{id}/lines",
    extensions(("x-permissions" = json!(["inventory.count"]))),
    operation_id = "put_inventory_count_lines",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the stock count.")),
    request_body = CountLinesRequest,
    responses(
        (status = 200, description = "The stock count.", body = Data<CountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.count` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No stock count (`inventory.count_not_found`) or variant (`inventory.variant_not_found`).", body = Problem),
        (status = 409, description = "The count is not open; code `inventory.invalid_transition`.", body = Problem),
        (status = 422, description = "A reason is not in the list (`kernel.reason_unknown`), a variant tracks no stock (`inventory.not_tracked`), or a value is invalid.", body = Problem),
    )
)]
async fn put_count_lines(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CountLinesRequest>,
) -> Result<Data<CountResponse>, Error> {
    principal.require(&COUNT)?;
    let lines = input
        .lines
        .into_iter()
        .map(|line| CountedLine {
            variant_id: line.variant_id,
            counted: line.counted,
            reason_code: line.reason_code,
        })
        .collect();
    Ok(Data { data: service.record_counts(&tenant, &principal, id, lines).await?.into() })
}

/// Record counted quantities from a CSV file.
///
/// Requires `inventory.count` covering the warehouse. Columns: `sku` (or barcode), `counted`,
/// optional `reason_code`. Errors name the CSV line; nothing is recorded unless every line is
/// valid.
#[utoipa::path(
    post,
    path = "/counts/{id}/import",
    extensions(("x-permissions" = json!(["inventory.count"]))),
    operation_id = "import_inventory_count",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the stock count.")),
    request_body = CountImportRequest,
    responses(
        (status = 200, description = "The stock count.", body = Data<CountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.count` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No stock count with this id in your scope; code `inventory.count_not_found`.", body = Problem),
        (status = 409, description = "The count is not open; code `inventory.invalid_transition`.", body = Problem),
        (status = 422, description = "The CSV is invalid; `errors` names each line.", body = Problem),
    )
)]
async fn import_count(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CountImportRequest>,
) -> Result<Data<CountResponse>, Error> {
    principal.require(&COUNT)?;
    Ok(Data { data: service.import_counts(&tenant, &principal, id, &input.csv).await?.into() })
}

/// Apply a stock count: each difference becomes a `count` ledger line with its reason.
///
/// Requires `inventory.adjust` covering the warehouse. The difference `counted −
/// expected_at_count` is added to the current on-hand quantity, so sales between counting and
/// applying are not counted twice.
#[utoipa::path(
    post,
    path = "/counts/{id}/apply",
    extensions(("x-permissions" = json!(["inventory.adjust"]))),
    operation_id = "apply_inventory_count",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the stock count.")),
    responses(
        (status = 200, description = "The stock count, now `applied`.", body = Data<CountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.adjust` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No stock count with this id in your scope; code `inventory.count_not_found`.", body = Problem),
        (status = 409, description = "The count is not open (`inventory.invalid_transition`) or on hand would go below 0 (`inventory.insufficient_stock`).", body = Problem),
    )
)]
async fn apply_count(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CountResponse>, Error> {
    principal.require(&ADJUST)?;
    Ok(Data { data: service.apply_count(&tenant, &principal, id).await?.into() })
}

/// Cancel an open stock count.
///
/// Requires `inventory.count` covering the warehouse.
#[utoipa::path(
    post,
    path = "/counts/{id}/cancel",
    extensions(("x-permissions" = json!(["inventory.count"]))),
    operation_id = "cancel_inventory_count",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the stock count.")),
    responses(
        (status = 200, description = "The stock count, now `cancelled`.", body = Data<CountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.count` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No stock count with this id in your scope; code `inventory.count_not_found`.", body = Problem),
        (status = 409, description = "The count is not open; code `inventory.invalid_transition`.", body = Problem),
    )
)]
async fn cancel_count(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CountResponse>, Error> {
    principal.require(&COUNT)?;
    Ok(Data { data: service.cancel_count(&tenant, &principal, id).await?.into() })
}
