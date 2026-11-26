//! Endpoint hoá đơn và chứng từ của đơn (D-215, B-c5): xin xuất hoá đơn (nhân viên / người mua),
//! danh sách và bản in PDF chứng từ cho người mua có tài khoản và cho token xem đơn vãng lai.

use autoxide_kernel::time::Date;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::entity::invoice_link;
use crate::guests::ORDER_TOKEN;
use crate::invoicing::InvoiceRequest;
use crate::service::OrdersService;

/// Company details for a VAT invoice.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersInvoiceRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct InvoiceRequestBody {
    /// Tax code of the company (10 or 13 digits, or a 12-digit personal id); omit for a consumer
    /// invoice.
    #[validate(length(min = 10, max = 14))]
    #[schema(example = "0101234567")]
    tax_code: Option<String>,
    /// Company name, required with `tax_code`.
    #[validate(length(min = 1, max = 200))]
    name: Option<String>,
    #[validate(length(min = 1, max = 400))]
    address: Option<String>,
    #[validate(email)]
    email: Option<String>,
}

impl From<InvoiceRequestBody> for InvoiceRequest {
    fn from(body: InvoiceRequestBody) -> Self {
        InvoiceRequest {
            tax_code: body.tax_code,
            name: body.name,
            address: body.address,
            email: body.email,
        }
    }
}

/// A document of an order (invoice or adjustment).
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersDocument)]
pub(crate) struct DocumentResponse {
    document_id: Uuid,
    /// `sales`, `cash_register` or `adjustment` (after a return).
    kind: String,
    /// The return of an adjustment.
    return_id: Option<Uuid>,
    /// Status at `invoices`: `issued`, `submitting`, `accepted`, `rejected`, `cancelled`,
    /// `replaced`, `adjusted`.
    status: String,
    /// Symbol and number printed on the invoice.
    symbol: Option<String>,
    number: Option<i64>,
    issue_date: Option<Date>,
    total: Option<Money>,
    /// Code to look the invoice up at the tax authority's portal.
    lookup_code: Option<String>,
    /// `true`: this invoice's tax or total differs from what was settled on the order — the invoice
    /// computes tax at its own date (the tax point), so a tax rate change between ordering and
    /// invoicing (e.g. the end of the 8% VAT on 2026-12-31) shows here. Check the invoice and settle
    /// the difference with the buyer. Always `false` for an adjustment, or when part of the order was
    /// cancelled.
    differs_from_order: bool,
}

fn document_response(
    settled: Option<(i64, i64)>,
    link: invoice_link::Model,
    document: Option<autoxide_invoices::api::Document>,
) -> DocumentResponse {
    let differs_from_order =
        crate::invoicing::differs_from_order(settled, &link.kind, document.as_ref());
    DocumentResponse {
        differs_from_order,
        document_id: link.document_id,
        kind: link.kind,
        return_id: link.return_id,
        status: document.as_ref().map_or(link.status, |document| document.status.clone()),
        symbol: document.as_ref().and_then(|document| document.symbol.clone()),
        number: document.as_ref().and_then(|document| document.number),
        issue_date: document.as_ref().and_then(|document| document.issue_date),
        total: document.as_ref().map(|document| document.total),
        lookup_code: document.and_then(|document| document.lookup_code),
    }
}

/// What an invoice request did.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersInvoiceRequested)]
pub(crate) struct InvoiceRequestedResponse {
    /// The invoice, when issued now; `null` when it will be issued at the time the shop's policy
    /// says (`orders.invoice_timing`).
    document: Option<DocumentResponse>,
}

async fn requested(
    service: &OrdersService,
    tenant: &TenantContext,
    row: crate::entity::order::Model,
    body: InvoiceRequestBody,
) -> Result<Data<InvoiceRequestedResponse>, Error> {
    let id = row.id;
    let link = service.request_invoice(tenant, row, body.into()).await?;
    let settled = service.settled_amounts(tenant, id).await?;
    let document = match link {
        Some(link) => service
            .order_document(tenant, id, link.id)
            .await?
            .map(|(link, document)| document_response(settled, link, document)),
        None => None,
    };
    Ok(Data { data: InvoiceRequestedResponse { document } })
}

/// Ask for the invoice of an order (staff).
///
/// Requires `orders.manage` at the order's shop. Saves the company details and issues the invoice
/// now when the order reached the time the shop's policy says (or the policy is `on_request`).
#[utoipa::path(
    post,
    path = "/{id}/invoice",
    extensions(("x-permissions" = json!(["orders.manage"]))),
    operation_id = "request_order_invoice",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    request_body = InvoiceRequestBody,
    responses(
        (status = 200, description = "Saved; the invoice when issued now.", body = Data<InvoiceRequestedResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.manage` at the shop; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such order (`orders.order_not_found`) or no invoicing here (`orders.invoices_unavailable`).", body = Problem),
        (status = 409, description = "The order already has an invoice; code `orders.invoice_already_issued`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn request_invoice(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<InvoiceRequestBody>,
) -> Result<Data<InvoiceRequestedResponse>, Error> {
    let row = service.managed_order(&tenant, &principal, id).await?;
    requested(&service, &tenant, row, input).await
}

/// Ask for the invoice of one of your orders.
///
/// Signed in: an order you placed. Same rules as the staff request.
#[utoipa::path(
    post,
    path = "/me/orders/{id}/invoice",
    extensions(("x-permissions" = json!([]))),
    operation_id = "request_my_order_invoice",
    params(("id" = Uuid, Path, description = "Id of the order.")),
    request_body = InvoiceRequestBody,
    responses(
        (status = 200, description = "Saved; the invoice when issued now.", body = Data<InvoiceRequestedResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such order of yours (`orders.order_not_found`) or no invoicing here (`orders.invoices_unavailable`).", body = Problem),
        (status = 409, description = "The order already has an invoice; code `orders.invoice_already_issued`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn request_my_invoice(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<InvoiceRequestBody>,
) -> Result<Data<InvoiceRequestedResponse>, Error> {
    let view = service.my_order(&tenant, principal.user_id(), id).await?;
    requested(&service, &tenant, view.order, input).await
}

async fn documents(
    service: &OrdersService,
    tenant: &TenantContext,
    id: Uuid,
    page: &PageRequest,
) -> Result<Page<DocumentResponse>, Error> {
    let found = service.order_documents(tenant, id, page).await?;
    let settled = service.settled_amounts(tenant, id).await?;
    Ok(found.map(|(link, document)| document_response(settled, link, document)))
}

/// Documents of one of your orders.
///
/// Signed in: an order you placed. Invoices and adjustments issued for it. Newest first unless
/// `order=asc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/me/orders/{id}/documents",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_order_documents",
    params(("id" = Uuid, Path, description = "Id of the order."), PageQuery),
    responses(
        (status = 200, description = "One page of the documents (empty when none yet).", body = Page<DocumentResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such order of yours; code `orders.order_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn my_documents(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<DocumentResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    service.my_order(&tenant, principal.user_id(), id).await?;
    documents(&service, &tenant, id, &page).await
}

/// Documents of a guest order.
///
/// Public with the `Order-Token` header: only the order the token opens. Newest first unless
/// `order=asc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/public/orders/{id}/documents",
    extensions(("x-public" = json!(true))),
    operation_id = "list_guest_order_documents",
    security(()),
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("Order-Token" = String, Header, description = "Access token of the order."),
        PageQuery,
    ),
    responses(
        (status = 200, description = "One page of the documents (empty when none yet).", body = Page<DocumentResponse>),
        (status = 401, description = "Missing, wrong or expired token; code `orders.lookup_token_invalid`.", body = Problem),
        (status = 404, description = "The token opens another order; code `orders.order_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn guest_documents(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<DocumentResponse>, Error> {
    let token = headers.get(ORDER_TOKEN).and_then(|value| value.to_str().ok());
    let access = service.guest_access(&tenant, token).await?;
    service.guest_order(&tenant, &access, id).await?;
    let page = PageRequest::try_from(page)?;
    documents(&service, &tenant, id, &page).await
}

fn pdf(rendered: autoxide_invoices::api::Rendered) -> Response {
    let disposition = format!("inline; filename=\"{}\"", rendered.file_name.replace('"', ""));
    let mut response = rendered.body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(rendered.content_type));
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// Print a document of one of your orders.
///
/// Signed in: an order you placed. Answers the PDF.
#[utoipa::path(
    get,
    path = "/me/orders/{id}/documents/{document_id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "print_my_order_document",
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("document_id" = Uuid, Path, description = "`document_id` from the document list."),
    ),
    responses(
        (status = 200, description = "The PDF.", content_type = "application/pdf", body = Vec<u8>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such order of yours (`orders.order_not_found`) or document of it (`orders.document_not_found`).", body = Problem),
    )
)]
pub(crate) async fn print_my_document(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, document_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, Error> {
    let view = service.my_order(&tenant, principal.user_id(), id).await?;
    let rendered = service.order_document_pdf(&tenant, id, document_id, &view.order.locale).await?;
    Ok(pdf(rendered))
}

/// Print a document of a guest order.
///
/// Public with the `Order-Token` header. Answers the PDF.
#[utoipa::path(
    get,
    path = "/public/orders/{id}/documents/{document_id}",
    extensions(("x-public" = json!(true))),
    operation_id = "print_guest_order_document",
    security(()),
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("document_id" = Uuid, Path, description = "`document_id` from the document list."),
        ("Order-Token" = String, Header, description = "Access token of the order."),
    ),
    responses(
        (status = 200, description = "The PDF.", content_type = "application/pdf", body = Vec<u8>),
        (status = 401, description = "Missing, wrong or expired token; code `orders.lookup_token_invalid`.", body = Problem),
        (status = 404, description = "The token opens another order (`orders.order_not_found`) or no such document (`orders.document_not_found`).", body = Problem),
    )
)]
pub(crate) async fn print_guest_document(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    headers: HeaderMap,
    Path((id, document_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, Error> {
    let token = headers.get(ORDER_TOKEN).and_then(|value| value.to_str().ok());
    let access = service.guest_access(&tenant, token).await?;
    let row = service.guest_order(&tenant, &access, id).await?;
    let rendered = service.order_document_pdf(&tenant, id, document_id, &row.locale).await?;
    Ok(pdf(rendered))
}
