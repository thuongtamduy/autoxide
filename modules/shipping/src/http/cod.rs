//! Đối soát tiền thu hộ (COD): bảng kê của hãng và chênh lệch (`shipping.cod.manage`).

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::api::CodStatement;
use crate::entity::{cod_discrepancy, cod_statement, cod_statement_line};
use crate::permissions::{COD_MANAGE, require_cod_read};
use crate::service::ShippingService;

/// A carrier statement to import: either the CSV file (columns `tracking_number`, `cod_amount`,
/// `fee` in minor units) or `fetch: true` to read it from the carrier's API.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ImportShippingCodStatement)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImportRequest {
    carrier_account_id: Uuid,
    /// The carrier's reference of the statement; importing it again with the same content does
    /// nothing.
    reference: Option<String>,
    period_from: Date,
    period_to: Date,
    /// ISO 4217 code of the amounts in the CSV (default `VND`).
    currency: Option<String>,
    /// The CSV text.
    csv: Option<String>,
    /// Read the statement from the carrier's API instead.
    #[serde(default)]
    fetch: bool,
}

/// A carrier statement.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingCodStatement)]
pub(crate) struct StatementBody {
    id: Uuid,
    carrier_account_id: Uuid,
    reference: String,
    period_from: Date,
    period_to: Date,
    total_cod: Money,
    total_fee: Money,
    net_paid: Money,
    /// `reconciled` or `has_mismatch`.
    status: String,
    imported_at: Timestamp,
}

/// A line of a carrier statement.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingCodStatementLine)]
pub(crate) struct LineBody {
    tracking_number: String,
    cod: Money,
    fee: Money,
    shipment_id: Option<Uuid>,
    /// `matched`, `amount_mismatch`, `unknown_tracking` or `not_delivered`.
    r#match: String,
}

/// A carrier statement with its lines.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingCodStatementDetail)]
pub(crate) struct StatementDetail {
    #[serde(flatten)]
    statement: StatementBody,
    lines: Vec<LineBody>,
}

fn money(minor: i64, currency: &str) -> Money {
    Money::from_columns(minor, currency).unwrap_or_else(|_| Money::new(minor, Currency::VND))
}

fn statement(row: cod_statement::Model) -> StatementBody {
    StatementBody {
        total_cod: money(row.total_cod_minor, &row.currency),
        total_fee: money(row.total_fee_minor, &row.currency),
        net_paid: money(row.net_paid_minor, &row.currency),
        id: row.id,
        carrier_account_id: row.carrier_account_id,
        reference: row.statement_reference,
        period_from: row.period_from,
        period_to: row.period_to,
        status: row.status,
        imported_at: row.imported_at,
    }
}

fn line(row: cod_statement_line::Model, currency: &str) -> LineBody {
    LineBody {
        cod: money(row.cod_minor, currency),
        fee: money(row.fee_minor, currency),
        tracking_number: row.tracking_number,
        shipment_id: row.shipment_id,
        r#match: row.match_kind,
    }
}

/// Import a cash-on-delivery statement of a carrier.
///
/// Needs `shipping.cod.manage`. Each line is matched with a shipment by tracking number: matching
/// lines settle the shipment (ledger entry, event `shipping.cod_settled`); others open a
/// discrepancy (event `shipping.cod_mismatch`). The same reference with the same content answers
/// the existing statement (200).
#[utoipa::path(
    post,
    path = "/cod/statements",
    extensions(("x-permissions" = json!(["shipping.cod.manage"]))),
    operation_id = "import_shipping_cod_statement",
    security(("bearer" = [])),
    request_body = ImportRequest,
    responses(
        (status = 201, description = "Imported.", body = Data<StatementBody>),
        (status = 200, description = "Already imported with the same content.", body = Data<StatementBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.cod.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No carrier account with this id; code `shipping.carrier_account_not_found`.", body = Problem),
        (status = 409, description = "The reference was imported with other content; code `shipping.statement_duplicate`.", body = Problem),
        (status = 422, description = "The CSV or a value is invalid (see `errors`), or the carrier has no statement API (`shipping.statement_not_available`).", body = Problem),
        (status = 503, description = "The carrier did not answer; code `shipping.carrier_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn import_statement(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ImportRequest>,
) -> Result<axum::response::Response, Error> {
    principal.require(&COD_MANAGE)?;
    let (row, created) = if input.fetch {
        service
            .fetch_statement(
                &tenant,
                &principal,
                input.carrier_account_id,
                input.period_from,
                input.period_to,
            )
            .await?
    } else {
        let currency = Currency::parse(input.currency.as_deref().unwrap_or("VND"))?;
        let lines = crate::cod::parse_csv(input.csv.as_deref().unwrap_or_default(), currency)?;
        let reference = input
            .reference
            .unwrap_or_else(|| format!("{}..{}", input.period_from, input.period_to));
        let statement = CodStatement::new(reference, input.period_from, input.period_to, lines);
        service.import_statement(&tenant, &principal, input.carrier_account_id, statement).await?
    };
    let id = row.id;
    let body = statement(row);
    Ok(if created {
        Created::new(format!("/shipping/cod/statements/{id}"), body).into_response()
    } else {
        Data { data: body }.into_response()
    })
}

/// List imported cash-on-delivery statements.
#[utoipa::path(
    get,
    path = "/cod/statements",
    extensions(("x-permissions" = json!(["shipping.cod.read", "shipping.cod.manage"]))),
    operation_id = "list_shipping_cod_statements",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of statements.", body = Page<StatementBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.cod.read` or `shipping.cod.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_statements(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<StatementBody>, Error> {
    require_cod_read(&principal)?;
    let page = PageRequest::try_from(page)?;
    Ok(service.list_statements(&tenant, &page).await?.map(statement))
}

/// Get a cash-on-delivery statement with its lines.
#[utoipa::path(
    get,
    path = "/cod/statements/{id}",
    extensions(("x-permissions" = json!(["shipping.cod.read", "shipping.cod.manage"]))),
    operation_id = "get_shipping_cod_statement",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the statement.")),
    responses(
        (status = 200, description = "The statement.", body = Data<StatementDetail>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.cod.read` or `shipping.cod.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No statement with this id; code `shipping.statement_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_statement(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<StatementDetail>, Error> {
    require_cod_read(&principal)?;
    let (row, lines) = service.get_statement(&tenant, id).await?;
    let currency = row.currency.clone();
    Ok(Data {
        data: StatementDetail {
            statement: statement(row),
            lines: lines.into_iter().map(|row| line(row, &currency)).collect(),
        },
    })
}

/// Filter of discrepancies.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DiscrepancyQuery {
    /// `open` or `resolved`.
    status: Option<String>,
}

/// A difference between a carrier statement and the shipments.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingCodDiscrepancy)]
pub(crate) struct DiscrepancyBody {
    id: Uuid,
    /// `amount_mismatch`, `unknown_tracking`, `not_delivered` or `overdue`.
    kind: String,
    shipment_id: Option<Uuid>,
    statement_line_id: Option<Uuid>,
    /// Amount of the shipment request.
    expected: Money,
    /// Amount on the statement.
    reported: Money,
    /// `open` or `resolved`.
    status: String,
    resolution_code: Option<String>,
    resolution_note: String,
    resolved_by: Option<Uuid>,
    resolved_at: Option<Timestamp>,
    created_at: Timestamp,
}

pub(crate) fn discrepancy(row: cod_discrepancy::Model) -> DiscrepancyBody {
    DiscrepancyBody {
        expected: money(row.expected_minor, &row.currency),
        reported: money(row.reported_minor, &row.currency),
        id: row.id,
        kind: row.kind,
        shipment_id: row.shipment_id,
        statement_line_id: row.statement_line_id,
        status: row.status,
        resolution_code: row.resolution_code,
        resolution_note: row.resolution_note,
        resolved_by: row.resolved_by,
        resolved_at: row.resolved_at,
        created_at: row.created_at,
    }
}

/// List cash-on-delivery discrepancies.
#[utoipa::path(
    get,
    path = "/cod/discrepancies",
    extensions(("x-permissions" = json!(["shipping.cod.read", "shipping.cod.manage"]))),
    operation_id = "list_shipping_cod_discrepancies",
    security(("bearer" = [])),
    params(DiscrepancyQuery, PageQuery),
    responses(
        (status = 200, description = "One page of discrepancies.", body = Page<DiscrepancyBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.cod.read` or `shipping.cod.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_discrepancies(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<DiscrepancyQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<DiscrepancyBody>, Error> {
    require_cod_read(&principal)?;
    let page = PageRequest::try_from(page)?;
    Ok(service.list_discrepancies(&tenant, query.status, &page).await?.map(discrepancy))
}

/// How a discrepancy was resolved.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ResolveShippingCodDiscrepancy)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolveRequest {
    /// An active code of the reason list `shipping.cod_resolution` (`carrier_corrected`,
    /// `delivered_confirmed`, `returned_confirmed`, `written_off`, or the tenant's own).
    resolution_code: String,
    /// Required (1 to 1000 characters): what was checked.
    note: String,
}

/// Resolve a cash-on-delivery discrepancy.
///
/// Needs `shipping.cod.manage`. Recorded with who resolved it; `written_off` writes the amount
/// off in the ledger.
#[utoipa::path(
    post,
    path = "/cod/discrepancies/{id}/resolve",
    extensions(("x-permissions" = json!(["shipping.cod.manage"]))),
    operation_id = "resolve_shipping_cod_discrepancy",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the discrepancy.")),
    request_body = ResolveRequest,
    responses(
        (status = 200, description = "Resolved.", body = Data<DiscrepancyBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.cod.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No discrepancy with this id; code `shipping.discrepancy_not_found`.", body = Problem),
        (status = 409, description = "Already resolved; code `shipping.discrepancy_resolved`.", body = Problem),
        (status = 422, description = "Unknown resolution code or missing note; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn resolve_discrepancy(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ResolveRequest>,
) -> Result<Data<DiscrepancyBody>, Error> {
    principal.require(&COD_MANAGE)?;
    let row = service
        .resolve_discrepancy(&tenant, &principal, id, input.resolution_code, input.note)
        .await?;
    Ok(Data { data: discrepancy(row) })
}

/// Ask the carrier again for the tracking history of a discrepancy's shipment.
///
/// Needs `shipping.cod.manage`. Calls the carrier (outside any write transaction) and applies new
/// tracking steps to the shipment.
#[utoipa::path(
    post,
    path = "/cod/discrepancies/{id}/refresh-tracking",
    extensions(("x-permissions" = json!(["shipping.cod.manage"]))),
    operation_id = "refresh_shipping_cod_discrepancy_tracking",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the discrepancy.")),
    responses(
        (status = 200, description = "The discrepancy (the shipment was refreshed).", body = Data<DiscrepancyBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.cod.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No discrepancy with this id; code `shipping.discrepancy_not_found`.", body = Problem),
        (status = 503, description = "The carrier did not answer; code `shipping.carrier_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn refresh_tracking(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<DiscrepancyBody>, Error> {
    principal.require(&COD_MANAGE)?;
    let row = service.refresh_discrepancy(&tenant, id).await?;
    Ok(Data { data: discrepancy(row) })
}
