//! Endpoint nhập hàng: nhà cung cấp, đơn đặt hàng, phiếu nhập, trả hàng nhà cung cấp.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{PURCHASING_MANAGE, PURCHASING_READ, READ, RECEIVE};
use crate::purchasing::{
    PurchaseLineInput, PurchaseOrder, PurchaseOrderInput, Supplier, SupplierInput, status,
};
use crate::receipts::{
    Receipt, ReceiptInput, ReceiptLineInput, ReturnInput, ReturnLineInput, SupplierReturn,
};
use crate::service::InventoryService;

pub(crate) fn routes(service: Arc<InventoryService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_suppliers, create_supplier))
        .routes(routes!(get_supplier, update_supplier))
        .routes(routes!(list_purchase_orders, create_purchase_order))
        .routes(routes!(get_purchase_order, update_purchase_order))
        .routes(routes!(order_purchase_order))
        .routes(routes!(close_purchase_order))
        .routes(routes!(cancel_purchase_order))
        .routes(routes!(list_receipts, create_receipt))
        .routes(routes!(get_receipt))
        .routes(routes!(retry_receipt_ledger))
        .routes(routes!(list_supplier_returns, create_supplier_return))
        .routes(routes!(get_supplier_return))
        .with_state(service)
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A supplier.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventorySupplier)]
pub(crate) struct SupplierResponse {
    pub id: Uuid,
    /// The purchasing unit; the supplier's payable is kept in its ledger book.
    pub org_id: Uuid,
    pub code: String,
    pub name: String,
    pub tax_code: Option<String>,
    pub contact_name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address: Option<String>,
    pub payment_terms_days: i32,
    pub currency: String,
    pub active: bool,
    /// What the unit owes the supplier (from module `ledger`); absent when `ledger` is not
    /// installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payable: Option<Money>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Supplier> for SupplierResponse {
    fn from(found: Supplier) -> Self {
        let row = found.row;
        SupplierResponse {
            id: row.id,
            org_id: row.org_id,
            code: row.code,
            name: row.name,
            tax_code: row.tax_code,
            contact_name: row.contact_name,
            phone: row.phone,
            email: row.email,
            address: row.address,
            payment_terms_days: row.payment_terms_days,
            currency: row.currency,
            active: row.active,
            payable: found.payable,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A supplier to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventorySupplierRequest)]
pub(crate) struct SupplierRequest {
    /// The purchasing unit; default the tenant's root unit.
    pub org_id: Option<Uuid>,
    /// Your code, unique in the tenant, up to 64 characters.
    pub code: String,
    /// Up to 200 characters.
    pub name: String,
    /// Tax registration number, up to 20 characters.
    pub tax_code: Option<String>,
    pub contact_name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    /// Postal address as text, up to 500 characters.
    pub address: Option<String>,
    /// Days to pay, 0 to 365; default 0.
    pub payment_terms_days: Option<i32>,
    /// ISO 4217 code; default `VND`.
    pub currency: Option<String>,
}

/// Changes to a supplier; `null` clears an optional field.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventorySupplierUpdate)]
pub(crate) struct SupplierUpdate {
    pub code: Option<String>,
    pub name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub tax_code: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub contact_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub phone: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub email: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    pub address: Option<Option<String>>,
    pub payment_terms_days: Option<i32>,
    pub currency: Option<String>,
    pub active: Option<bool>,
}

/// Filters of the supplier list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct SuppliersQuery {
    /// Words of the name, with or without Vietnamese accents.
    pub q: Option<String>,
    pub active: Option<bool>,
}

/// List suppliers in your scope with what you owe each one.
///
/// Requires `inventory.purchasing.read`. `payable` comes from module `ledger` in one call for the
/// whole page.
#[utoipa::path(
    get,
    path = "/suppliers",
    extensions(("x-permissions" = json!(["inventory.purchasing.read"]))),
    operation_id = "list_inventory_suppliers",
    security(("bearer" = [])),
    params(SuppliersQuery, PageQuery),
    responses(
        (status = 200, description = "One page of suppliers.", body = Page<SupplierResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_suppliers(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<SuppliersQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<SupplierResponse>, Error> {
    principal.require(&PURCHASING_READ)?;
    let page = PageRequest::try_from(page)?;
    let found = service
        .list_suppliers(&tenant, &principal, query.q.as_deref(), query.active, &page)
        .await?;
    Ok(found.map(SupplierResponse::from))
}

/// Create a supplier.
///
/// Requires `inventory.purchasing.manage` on the unit and the plan feature `inventory.purchasing`.
#[utoipa::path(
    post,
    path = "/suppliers",
    extensions(("x-permissions" = json!(["inventory.purchasing.manage"]))),
    operation_id = "create_inventory_supplier",
    security(("bearer" = [])),
    request_body = SupplierRequest,
    responses(
        (status = 201, description = "The supplier.", body = Data<SupplierResponse>, headers(("Location" = String, description = "Path of the supplier."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`), or the plan lacks purchasing (`feature_not_enabled`).", body = Problem),
        (status = 409, description = "The code is used already; code `inventory.supplier_code_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_supplier(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<SupplierRequest>,
) -> Result<Created<SupplierResponse>, Error> {
    principal.require(&PURCHASING_MANAGE)?;
    let input = SupplierInput {
        org_id: input.org_id,
        code: Some(input.code),
        name: Some(input.name),
        tax_code: Some(input.tax_code),
        contact_name: Some(input.contact_name),
        phone: Some(input.phone),
        email: Some(input.email),
        address: Some(input.address),
        payment_terms_days: input.payment_terms_days,
        currency: input.currency,
        active: None,
    };
    let saved =
        SupplierResponse::from(service.save_supplier(&tenant, &principal, None, input).await?);
    Ok(Created::new(format!("/inventory/suppliers/{}", saved.id), saved))
}

/// Get a supplier with what you owe it.
///
/// Requires `inventory.purchasing.read` covering the supplier's unit.
#[utoipa::path(
    get,
    path = "/suppliers/{id}",
    extensions(("x-permissions" = json!(["inventory.purchasing.read"]))),
    operation_id = "get_inventory_supplier",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the supplier.")),
    responses(
        (status = 200, description = "The supplier.", body = Data<SupplierResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No supplier with this id in your scope; code `inventory.supplier_not_found`.", body = Problem),
    )
)]
async fn get_supplier(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<SupplierResponse>, Error> {
    principal.require(&PURCHASING_READ)?;
    Ok(Data { data: service.get_supplier(&tenant, &principal, id).await?.into() })
}

/// Change a supplier.
///
/// Requires `inventory.purchasing.manage` covering the supplier's unit.
#[utoipa::path(
    patch,
    path = "/suppliers/{id}",
    extensions(("x-permissions" = json!(["inventory.purchasing.manage"]))),
    operation_id = "update_inventory_supplier",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the supplier.")),
    request_body = SupplierUpdate,
    responses(
        (status = 200, description = "The supplier.", body = Data<SupplierResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.manage` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No supplier with this id in your scope; code `inventory.supplier_not_found`.", body = Problem),
        (status = 409, description = "The code is used already; code `inventory.supplier_code_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_supplier(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SupplierUpdate>,
) -> Result<Data<SupplierResponse>, Error> {
    principal.require(&PURCHASING_MANAGE)?;
    let input = SupplierInput {
        org_id: None,
        code: input.code,
        name: input.name,
        tax_code: input.tax_code,
        contact_name: input.contact_name,
        phone: input.phone,
        email: input.email,
        address: input.address,
        payment_terms_days: input.payment_terms_days,
        currency: input.currency,
        active: input.active,
    };
    Ok(Data { data: service.save_supplier(&tenant, &principal, Some(id), input).await?.into() })
}

/// A variant, a quantity and a unit cost.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryCostLine)]
pub(crate) struct CostLine {
    pub variant_id: Uuid,
    /// 1 to 1 000 000.
    pub quantity: i64,
    /// Unit cost; every line in the same currency.
    pub unit_cost: Money,
}

/// One line of a purchase order.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryPurchaseOrderLine)]
pub(crate) struct PurchaseOrderLineResponse {
    pub variant_id: Uuid,
    pub quantity: i64,
    pub received_quantity: i64,
    pub unit_cost: Money,
}

/// A purchase order to a supplier.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryPurchaseOrder)]
pub(crate) struct PurchaseOrderResponse {
    pub id: Uuid,
    /// Document number, e.g. `PO-2026-000001`.
    pub number: String,
    /// The receiving warehouse.
    pub org_id: Uuid,
    pub supplier_id: Uuid,
    /// `draft`, `ordered`, `partially_received`, `received`, `closed` or `cancelled`.
    pub status: String,
    pub expected_on: Option<NaiveDate>,
    pub total: Money,
    pub note: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lines: Vec<PurchaseOrderLineResponse>,
}

impl TryFrom<PurchaseOrder> for PurchaseOrderResponse {
    type Error = Error;

    fn try_from(order: PurchaseOrder) -> Result<Self, Error> {
        let row = order.row;
        let mut lines = Vec::with_capacity(order.lines.len());
        for line in order.lines {
            lines.push(PurchaseOrderLineResponse {
                variant_id: line.variant_id,
                quantity: line.quantity,
                received_quantity: line.received_quantity,
                unit_cost: Money::from_columns(line.unit_cost_minor, &line.unit_cost_currency)?,
            });
        }
        Ok(PurchaseOrderResponse {
            id: row.id,
            number: row.number,
            org_id: row.org_id,
            supplier_id: row.supplier_id,
            status: row.status,
            expected_on: row.expected_on,
            total: Money::from_columns(row.total_minor, &row.currency)?,
            note: row.note,
            created_at: row.created_at,
            updated_at: row.updated_at,
            lines,
        })
    }
}

/// A purchase order to create or change (while a draft).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryPurchaseOrderRequest)]
pub(crate) struct PurchaseOrderRequest {
    /// The receiving warehouse.
    pub org_id: Uuid,
    pub supplier_id: Uuid,
    pub expected_on: Option<NaiveDate>,
    /// Up to 500 characters.
    pub note: Option<String>,
    /// 1 to 500 lines, one per variant.
    pub lines: Vec<CostLine>,
}

impl From<PurchaseOrderRequest> for PurchaseOrderInput {
    fn from(input: PurchaseOrderRequest) -> Self {
        PurchaseOrderInput {
            org_id: input.org_id,
            supplier_id: input.supplier_id,
            expected_on: input.expected_on,
            note: input.note,
            lines: input
                .lines
                .into_iter()
                .map(|line| PurchaseLineInput {
                    variant_id: line.variant_id,
                    quantity: line.quantity,
                    unit_cost: line.unit_cost,
                })
                .collect(),
        }
    }
}

/// Filters of the purchase order list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct PurchaseOrdersQuery {
    pub supplier_id: Option<Uuid>,
    /// `draft`, `ordered`, `partially_received`, `received`, `closed` or `cancelled`.
    pub status: Option<String>,
}

/// List purchase orders in your scope.
///
/// Requires `inventory.purchasing.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/purchase-orders",
    extensions(("x-permissions" = json!(["inventory.purchasing.read"]))),
    operation_id = "list_inventory_purchase_orders",
    security(("bearer" = [])),
    params(PurchaseOrdersQuery, PageQuery),
    responses(
        (status = 200, description = "One page of purchase orders.", body = Page<PurchaseOrderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_purchase_orders(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PurchaseOrdersQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<PurchaseOrderResponse>, Error> {
    principal.require(&PURCHASING_READ)?;
    let page = PageRequest::try_from(page)?;
    let found = service
        .list_purchase_orders(
            &tenant,
            &principal,
            query.supplier_id,
            query.status.as_deref(),
            &page,
        )
        .await?;
    let mut data = Vec::with_capacity(found.data.len());
    for order in found.data {
        data.push(PurchaseOrderResponse::try_from(order)?);
    }
    Ok(Page { data, pagination: found.pagination })
}

/// Create a purchase order as a draft.
///
/// Requires `inventory.purchasing.manage` covering the receiving warehouse and the plan feature
/// `inventory.purchasing`. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/purchase-orders",
    extensions(("x-permissions" = json!(["inventory.purchasing.manage"]))),
    operation_id = "create_inventory_purchase_order",
    security(("bearer" = [])),
    request_body = PurchaseOrderRequest,
    responses(
        (status = 201, description = "The draft purchase order.", body = Data<PurchaseOrderResponse>, headers(("Location" = String, description = "Path of the purchase order."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`), or the plan lacks purchasing (`feature_not_enabled`).", body = Problem),
        (status = 404, description = "No such supplier (`inventory.supplier_not_found`) or variant (`inventory.variant_not_found`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), the unit holds no stock (`inventory.not_a_warehouse`) or a variant tracks no stock (`inventory.not_tracked`).", body = Problem),
    )
)]
async fn create_purchase_order(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PurchaseOrderRequest>,
) -> Result<Created<PurchaseOrderResponse>, Error> {
    principal.require(&PURCHASING_MANAGE)?;
    let order = service.create_purchase_order(&tenant, &principal, input.into()).await?;
    let order = PurchaseOrderResponse::try_from(order)?;
    Ok(Created::new(format!("/inventory/purchase-orders/{}", order.id), order))
}

/// Get a purchase order.
///
/// Requires `inventory.purchasing.read` covering the receiving warehouse.
#[utoipa::path(
    get,
    path = "/purchase-orders/{id}",
    extensions(("x-permissions" = json!(["inventory.purchasing.read"]))),
    operation_id = "get_inventory_purchase_order",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the purchase order.")),
    responses(
        (status = 200, description = "The purchase order.", body = Data<PurchaseOrderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No purchase order with this id in your scope; code `inventory.purchase_order_not_found`.", body = Problem),
    )
)]
async fn get_purchase_order(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PurchaseOrderResponse>, Error> {
    principal.require(&PURCHASING_READ)?;
    let order = service.get_purchase_order(&tenant, &principal, id).await?;
    Ok(Data { data: order.try_into()? })
}

/// Change a draft purchase order (its lines are replaced).
///
/// Requires `inventory.purchasing.manage` covering the receiving warehouse. The warehouse and
/// supplier cannot change.
#[utoipa::path(
    patch,
    path = "/purchase-orders/{id}",
    extensions(("x-permissions" = json!(["inventory.purchasing.manage"]))),
    operation_id = "update_inventory_purchase_order",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the purchase order.")),
    request_body = PurchaseOrderRequest,
    responses(
        (status = 200, description = "The purchase order.", body = Data<PurchaseOrderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.manage` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No purchase order with this id in your scope; code `inventory.purchase_order_not_found`.", body = Problem),
        (status = 409, description = "Not a draft; code `inventory.invalid_transition`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_purchase_order(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<PurchaseOrderRequest>,
) -> Result<Data<PurchaseOrderResponse>, Error> {
    principal.require(&PURCHASING_MANAGE)?;
    let order = service.update_purchase_order(&tenant, &principal, id, input.into()).await?;
    Ok(Data { data: order.try_into()? })
}

async fn move_order(
    service: &InventoryService,
    tenant: &TenantContext,
    principal: &Principal,
    id: Uuid,
    to: &'static str,
) -> Result<Data<PurchaseOrderResponse>, Error> {
    principal.require(&PURCHASING_MANAGE)?;
    let order = service.move_purchase_order(tenant, principal, id, to).await?;
    Ok(Data { data: order.try_into()? })
}

/// Send a draft purchase order to the supplier: its units count as `incoming`.
///
/// Requires `inventory.purchasing.manage` covering the receiving warehouse.
#[utoipa::path(
    post,
    path = "/purchase-orders/{id}/order",
    extensions(("x-permissions" = json!(["inventory.purchasing.manage"]))),
    operation_id = "order_inventory_purchase_order",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the purchase order.")),
    responses(
        (status = 200, description = "The purchase order, now `ordered`.", body = Data<PurchaseOrderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.manage` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No purchase order with this id in your scope; code `inventory.purchase_order_not_found`.", body = Problem),
        (status = 409, description = "Not a draft; code `inventory.invalid_transition`.", body = Problem),
    )
)]
async fn order_purchase_order(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PurchaseOrderResponse>, Error> {
    move_order(&service, &tenant, &principal, id, status::ORDERED).await
}

/// Close a purchase order: what has not arrived stops counting as `incoming`.
///
/// Requires `inventory.purchasing.manage` covering the receiving warehouse.
#[utoipa::path(
    post,
    path = "/purchase-orders/{id}/close",
    extensions(("x-permissions" = json!(["inventory.purchasing.manage"]))),
    operation_id = "close_inventory_purchase_order",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the purchase order.")),
    responses(
        (status = 200, description = "The purchase order, now `closed`.", body = Data<PurchaseOrderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.manage` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No purchase order with this id in your scope; code `inventory.purchase_order_not_found`.", body = Problem),
        (status = 409, description = "Not ordered; code `inventory.invalid_transition`.", body = Problem),
    )
)]
async fn close_purchase_order(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PurchaseOrderResponse>, Error> {
    move_order(&service, &tenant, &principal, id, status::CLOSED).await
}

/// Cancel a purchase order that is a draft, or ordered with nothing received.
///
/// Requires `inventory.purchasing.manage` covering the receiving warehouse.
#[utoipa::path(
    post,
    path = "/purchase-orders/{id}/cancel",
    extensions(("x-permissions" = json!(["inventory.purchasing.manage"]))),
    operation_id = "cancel_inventory_purchase_order",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the purchase order.")),
    responses(
        (status = 200, description = "The purchase order, now `cancelled`.", body = Data<PurchaseOrderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.manage` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No purchase order with this id in your scope; code `inventory.purchase_order_not_found`.", body = Problem),
        (status = 409, description = "Goods were received already; code `inventory.invalid_transition`.", body = Problem),
    )
)]
async fn cancel_purchase_order(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PurchaseOrderResponse>, Error> {
    move_order(&service, &tenant, &principal, id, status::CANCELLED).await
}

/// One line of a goods receipt.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryReceiptLineRequest)]
pub(crate) struct ReceiptLineRequest {
    pub variant_id: Uuid,
    /// 1 to 1 000 000.
    pub quantity: i64,
    /// Unit cost; every line in the same currency. It updates the average cost.
    pub unit_cost: Money,
    /// Required when the variant tracks lots.
    pub lot_code: Option<String>,
    pub expires_on: Option<NaiveDate>,
    /// Exactly `quantity` serial numbers when the variant tracks serials.
    #[serde(default)]
    pub serials: Vec<String>,
}

/// A goods receipt.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryReceiptRequest)]
pub(crate) struct ReceiptRequest {
    /// The receiving warehouse.
    pub org_id: Uuid,
    /// The supplier, for a receipt without a purchase order.
    pub supplier_id: Option<Uuid>,
    /// The purchase order received (the supplier comes from it).
    pub purchase_order_id: Option<Uuid>,
    /// Your id of this receipt (e.g. the delivery note number); the same reference and content
    /// again returns the receipt already made.
    pub reference: String,
    pub lines: Vec<ReceiptLineRequest>,
}

/// One line of a goods receipt.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryReceiptLine)]
pub(crate) struct ReceiptLineResponse {
    pub variant_id: Uuid,
    pub quantity: i64,
    pub unit_cost: Money,
    pub lot_code: Option<String>,
    pub expires_on: Option<NaiveDate>,
    pub serials: Vec<String>,
}

/// A goods receipt.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventoryReceipt)]
pub(crate) struct ReceiptResponse {
    pub id: Uuid,
    /// Document number, e.g. `PN-2026-000001`.
    pub number: String,
    pub org_id: Uuid,
    pub supplier_id: Option<Uuid>,
    pub purchase_order_id: Option<Uuid>,
    pub reference: String,
    pub posted_at: DateTime<Utc>,
    pub total: Money,
    /// `pending`, `posted`, `failed` (retry with `POST /receipts/{id}/retry-ledger`) or
    /// `not_applicable` (no supplier, nothing to pay, or module `ledger` not installed).
    pub ledger_status: String,
    pub ledger_entry_id: Option<Uuid>,
    pub ledger_posted_at: Option<DateTime<Utc>>,
    pub ledger_error_code: Option<String>,
    pub lines: Vec<ReceiptLineResponse>,
}

impl TryFrom<Receipt> for ReceiptResponse {
    type Error = Error;

    fn try_from(found: Receipt) -> Result<Self, Error> {
        let row = found.row;
        let mut lines = Vec::with_capacity(found.lines.len());
        for line in found.lines {
            lines.push(ReceiptLineResponse {
                variant_id: line.variant_id,
                quantity: line.quantity,
                unit_cost: Money::from_columns(line.unit_cost_minor, &line.unit_cost_currency)?,
                lot_code: line.lot_code,
                expires_on: line.expires_on,
                serials: serde_json::from_str(&line.serials).unwrap_or_default(),
            });
        }
        Ok(ReceiptResponse {
            id: row.id,
            number: row.number,
            org_id: row.org_id,
            supplier_id: row.supplier_id,
            purchase_order_id: row.purchase_order_id,
            reference: row.reference,
            posted_at: row.posted_at,
            total: Money::from_columns(row.total_minor, &row.currency)?,
            ledger_status: row.ledger_status,
            ledger_entry_id: row.ledger_entry_id,
            ledger_posted_at: row.ledger_posted_at,
            ledger_error_code: row.ledger_error_code,
            lines,
        })
    }
}

/// Filter of the receipt list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ReceiptsQuery {
    pub supplier_id: Option<Uuid>,
}

/// List goods receipts in your scope.
///
/// Requires `inventory.receive`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/receipts",
    extensions(("x-permissions" = json!(["inventory.receive"]))),
    operation_id = "list_inventory_receipts",
    security(("bearer" = [])),
    params(ReceiptsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of goods receipts.", body = Page<ReceiptResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.receive`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_receipts(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ReceiptsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReceiptResponse>, Error> {
    principal.require(&RECEIVE)?;
    let page = PageRequest::try_from(page)?;
    let found = service.list_receipts(&tenant, &principal, query.supplier_id, &page).await?;
    let mut data = Vec::with_capacity(found.data.len());
    for receipt in found.data {
        data.push(ReceiptResponse::try_from(receipt)?);
    }
    Ok(Page { data, pagination: found.pagination })
}

fn receipt_input(input: ReceiptRequest) -> ReceiptInput {
    ReceiptInput {
        org_id: input.org_id,
        supplier_id: input.supplier_id,
        purchase_order_id: input.purchase_order_id,
        reference: input.reference,
        lines: input
            .lines
            .into_iter()
            .map(|line| ReceiptLineInput {
                variant_id: line.variant_id,
                quantity: line.quantity,
                unit_cost: line.unit_cost,
                lot_code: line.lot_code,
                expires_on: line.expires_on,
                serials: line.serials,
            })
            .collect(),
    }
}

/// Receive goods into a warehouse, against a purchase order or directly.
///
/// Requires `inventory.receive` covering the warehouse (and the plan feature
/// `inventory.purchasing` when a supplier or purchase order is given). Each line updates the
/// average cost; a purchase order can be received in parts, never beyond what was ordered. With
/// a supplier and module `ledger`, a job posts Debit inventory / Credit supplier payable. Send the
/// same `reference` and content again to get the receipt already made (`200`).
#[utoipa::path(
    post,
    path = "/receipts",
    extensions(("x-permissions" = json!(["inventory.receive"]))),
    operation_id = "create_inventory_receipt",
    security(("bearer" = [])),
    request_body = ReceiptRequest,
    responses(
        (status = 200, description = "The receipt was made before with this `reference` and content.", body = Data<ReceiptResponse>),
        (status = 201, description = "The receipt.", body = Data<ReceiptResponse>, headers(("Location" = String, description = "Path of the receipt."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.receive` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`), or the plan does not allow it (`feature_not_enabled`, `limit_exceeded`).", body = Problem),
        (status = 404, description = "No such supplier, purchase order or variant (`inventory.supplier_not_found`, `inventory.purchase_order_not_found`, `inventory.variant_not_found`).", body = Problem),
        (status = 409, description = "More than ordered (`inventory.over_receipt`), the order is not open (`inventory.invalid_transition`), or a serial number is in stock already (`inventory.serial_unavailable`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), the cost currency differs (`inventory.currency_mismatch`), the unit holds no stock (`inventory.not_a_warehouse`), or the reference was used for other content (`reference_reused`).", body = Problem),
    )
)]
async fn create_receipt(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ReceiptRequest>,
) -> Result<Response, Error> {
    principal.require(&RECEIVE)?;
    Ok(match service.create_receipt(&tenant, &principal, receipt_input(input)).await? {
        Submitted::Created(receipt) => {
            let receipt = ReceiptResponse::try_from(receipt)?;
            Created::new(format!("/inventory/receipts/{}", receipt.id), receipt).into_response()
        }
        Submitted::Replayed(receipt) => {
            Data { data: ReceiptResponse::try_from(receipt)? }.into_response()
        }
    })
}

/// Get a goods receipt.
///
/// Requires `inventory.receive` or `inventory.read` covering the warehouse.
#[utoipa::path(
    get,
    path = "/receipts/{id}",
    extensions(("x-permissions" = json!(["inventory.receive"]))),
    operation_id = "get_inventory_receipt",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the receipt.")),
    responses(
        (status = 200, description = "The receipt.", body = Data<ReceiptResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.receive` and `inventory.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No receipt with this id in your scope; code `inventory.receipt_not_found`.", body = Problem),
    )
)]
async fn get_receipt(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ReceiptResponse>, Error> {
    if !principal.has(&READ) {
        principal.require(&RECEIVE)?;
    }
    let receipt = service.get_receipt(&tenant, &principal, id).await?;
    Ok(Data { data: receipt.try_into()? })
}

/// Post a receipt whose ledger posting failed again.
///
/// Requires `inventory.purchasing.manage` covering the warehouse. The receipt goes back to
/// `pending` and the posting job runs again.
#[utoipa::path(
    post,
    path = "/receipts/{id}/retry-ledger",
    extensions(("x-permissions" = json!(["inventory.purchasing.manage"]))),
    operation_id = "retry_inventory_receipt_ledger",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the receipt.")),
    responses(
        (status = 200, description = "The receipt, now `pending`.", body = Data<ReceiptResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.manage` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No receipt with this id in your scope; code `inventory.receipt_not_found`.", body = Problem),
        (status = 409, description = "The posting did not fail; code `inventory.invalid_transition`.", body = Problem),
    )
)]
async fn retry_receipt_ledger(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ReceiptResponse>, Error> {
    principal.require(&PURCHASING_MANAGE)?;
    let receipt = service.retry_receipt_ledger(&tenant, &principal, id).await?;
    Ok(Data { data: receipt.try_into()? })
}

/// One line of a supplier return.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventorySupplierReturnLineRequest)]
pub(crate) struct ReturnLineRequest {
    pub variant_id: Uuid,
    /// 1 to 1 000 000.
    pub quantity: i64,
    /// Unit price credited by the supplier; default the current average cost.
    pub unit_cost: Option<Money>,
    /// The lot returned; default the lots that expire first.
    pub lot_code: Option<String>,
    /// Exactly `quantity` serial numbers when the variant tracks serials.
    #[serde(default)]
    pub serials: Vec<String>,
}

/// A return of goods to a supplier.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventorySupplierReturnRequest)]
pub(crate) struct ReturnRequest {
    /// The warehouse the goods leave.
    pub org_id: Uuid,
    pub supplier_id: Uuid,
    /// The receipt the goods came with, if known.
    pub receipt_id: Option<Uuid>,
    /// Your id of this return; the same reference and content again returns it.
    pub reference: String,
    pub lines: Vec<ReturnLineRequest>,
}

/// One line of a supplier return.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventorySupplierReturnLine)]
pub(crate) struct ReturnLineResponse {
    pub variant_id: Uuid,
    pub quantity: i64,
    pub unit_cost: Money,
    pub lot_code: Option<String>,
    pub serials: Vec<String>,
}

/// A return of goods to a supplier.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = InventorySupplierReturn)]
pub(crate) struct ReturnResponse {
    pub id: Uuid,
    /// Document number, e.g. `TH-2026-000001`.
    pub number: String,
    pub org_id: Uuid,
    pub supplier_id: Uuid,
    pub receipt_id: Option<Uuid>,
    pub reference: String,
    pub posted_at: DateTime<Utc>,
    pub total: Money,
    /// `pending`, `posted`, `failed` or `not_applicable`.
    pub ledger_status: String,
    pub ledger_entry_id: Option<Uuid>,
    pub ledger_error_code: Option<String>,
    pub lines: Vec<ReturnLineResponse>,
}

impl TryFrom<SupplierReturn> for ReturnResponse {
    type Error = Error;

    fn try_from(found: SupplierReturn) -> Result<Self, Error> {
        let row = found.row;
        let mut lines = Vec::with_capacity(found.lines.len());
        for line in found.lines {
            lines.push(ReturnLineResponse {
                variant_id: line.variant_id,
                quantity: line.quantity,
                unit_cost: Money::from_columns(line.unit_cost_minor, &line.unit_cost_currency)?,
                lot_code: line.lot_code,
                serials: serde_json::from_str(&line.serials).unwrap_or_default(),
            });
        }
        Ok(ReturnResponse {
            id: row.id,
            number: row.number,
            org_id: row.org_id,
            supplier_id: row.supplier_id,
            receipt_id: row.receipt_id,
            reference: row.reference,
            posted_at: row.posted_at,
            total: Money::from_columns(row.total_minor, &row.currency)?,
            ledger_status: row.ledger_status,
            ledger_entry_id: row.ledger_entry_id,
            ledger_error_code: row.ledger_error_code,
            lines,
        })
    }
}

/// List supplier returns in your scope.
///
/// Requires `inventory.purchasing.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/supplier-returns",
    extensions(("x-permissions" = json!(["inventory.purchasing.read"]))),
    operation_id = "list_inventory_supplier_returns",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of supplier returns.", body = Page<ReturnResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_supplier_returns(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReturnResponse>, Error> {
    principal.require(&PURCHASING_READ)?;
    let page = PageRequest::try_from(page)?;
    let found = service.list_supplier_returns(&tenant, &principal, &page).await?;
    let mut data = Vec::with_capacity(found.data.len());
    for item in found.data {
        data.push(ReturnResponse::try_from(item)?);
    }
    Ok(Page { data, pagination: found.pagination })
}

/// Return goods to a supplier.
///
/// Requires `inventory.purchasing.manage` covering the warehouse. Takes only stock not reserved
/// for orders; with module `ledger`, a job reverses the receipt posting (Debit supplier payable /
/// Credit inventory). Send the same `reference` and content again to get the return already made
/// (`200`).
#[utoipa::path(
    post,
    path = "/supplier-returns",
    extensions(("x-permissions" = json!(["inventory.purchasing.manage"]))),
    operation_id = "create_inventory_supplier_return",
    security(("bearer" = [])),
    request_body = ReturnRequest,
    responses(
        (status = 200, description = "The return was made before with this `reference` and content.", body = Data<ReturnResponse>),
        (status = 201, description = "The return.", body = Data<ReturnResponse>, headers(("Location" = String, description = "Path of the return."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`), or the plan lacks purchasing (`feature_not_enabled`).", body = Problem),
        (status = 404, description = "No such supplier (`inventory.supplier_not_found`) or variant (`inventory.variant_not_found`).", body = Problem),
        (status = 409, description = "Not enough available stock (`inventory.insufficient_stock`), lot (`inventory.lot_unavailable`) or serial (`inventory.serial_unavailable`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`) or the reference was used for other content (`reference_reused`).", body = Problem),
    )
)]
async fn create_supplier_return(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ReturnRequest>,
) -> Result<Response, Error> {
    principal.require(&PURCHASING_MANAGE)?;
    let input = ReturnInput {
        org_id: input.org_id,
        supplier_id: input.supplier_id,
        receipt_id: input.receipt_id,
        reference: input.reference,
        lines: input
            .lines
            .into_iter()
            .map(|line| ReturnLineInput {
                variant_id: line.variant_id,
                quantity: line.quantity,
                unit_cost: line.unit_cost,
                lot_code: line.lot_code,
                serials: line.serials,
            })
            .collect(),
    };
    Ok(match service.create_supplier_return(&tenant, &principal, input).await? {
        Submitted::Created(found) => {
            let found = ReturnResponse::try_from(found)?;
            Created::new(format!("/inventory/supplier-returns/{}", found.id), found).into_response()
        }
        Submitted::Replayed(found) => {
            Data { data: ReturnResponse::try_from(found)? }.into_response()
        }
    })
}

/// Get a supplier return.
///
/// Requires `inventory.purchasing.read` covering the warehouse.
#[utoipa::path(
    get,
    path = "/supplier-returns/{id}",
    extensions(("x-permissions" = json!(["inventory.purchasing.read"]))),
    operation_id = "get_inventory_supplier_return",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the supplier return.")),
    responses(
        (status = 200, description = "The supplier return.", body = Data<ReturnResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.purchasing.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No supplier return with this id in your scope; code `inventory.supplier_return_not_found`.", body = Problem),
    )
)]
async fn get_supplier_return(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ReturnResponse>, Error> {
    principal.require(&PURCHASING_READ)?;
    let found = service.get_supplier_return(&tenant, &principal, id).await?;
    Ok(Data { data: found.try_into()? })
}
