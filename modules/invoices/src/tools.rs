//! Tool MCP của `invoices`: cùng service, cùng permission với REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::decimal::{DecimalString, Rounding};
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::{Buyer, Document, NewDocument, NewDocumentLine};
use crate::documents::DocumentFilter;
use crate::permissions::{ISSUE, READ, WRITE};
use crate::service::InvoicesService;

/// Which documents.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DocumentsInput {
    /// Only documents written by this unit.
    org_id: Option<Uuid>,
    kind: Option<String>,
    status: Option<String>,
    source_module: Option<String>,
    source_id: Option<String>,
    /// Maximum number of documents, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// Which document.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DocumentId {
    document_id: Uuid,
}

// Trường phẳng (client MCP không đọc `$ref`): dòng khai inline, tiền là chữ thập phân theo đơn vị
// chính trong `currency` của chứng từ.
/// One line of a draft.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LineInput {
    description: String,
    /// Decimal string, default `"1"`.
    quantity: Option<String>,
    /// Decimal string in the document currency, e.g. `"150000"`.
    unit_price: String,
    unit: Option<String>,
    /// Tax category, required on sales documents, e.g. `vat.standard`.
    tax_category: Option<String>,
}

/// A draft to write.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftInput {
    /// The unit; default the tenant's root.
    org_id: Option<Uuid>,
    /// e.g. `vat_invoice`, `sales_invoice`, `retail_receipt`.
    kind: String,
    /// e.g. `VND`.
    currency: String,
    /// `consumer` (default) or `business`.
    buyer_kind: Option<String>,
    buyer_name: Option<String>,
    buyer_tax_code: Option<String>,
    buyer_address: Option<String>,
    buyer_email: Option<String>,
    #[schema(inline)]
    lines: Vec<LineInput>,
    /// Your id of the draft: the same reference and content returns the same draft.
    reference: String,
}

pub(crate) fn all(service: &Arc<InvoicesService>) -> Vec<Tool> {
    let (list, get, create, issue) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "documents",
            "List accounting documents (invoices, receipts, notes) in your scope with their lines.",
            move |tenant: TenantContext, principal: Principal, input: DocumentsInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let filter = DocumentFilter {
                        org: input.org_id.map(OrgId::from_uuid),
                        kind: input.kind,
                        status: input.status,
                        source_module: input.source_module,
                        source_id: input.source_id,
                    };
                    let found: Page<Document> = service.list_documents(&tenant, &principal, filter, &page).await?;
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "document",
            "An accounting document with its lines, status, number and e-invoice codes.",
            move |tenant: TenantContext, principal: Principal, input: DocumentId| {
                let service = get.clone();
                async move { service.document_detail(&tenant, &principal, input.document_id).await }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "create_draft",
            "Write a draft document; taxes are computed from the tax categories. A draft has no number \
             until it is issued.",
            move |tenant: TenantContext, principal: Principal, input: DraftInput| {
                let service = create.clone();
                async move {
                    let currency = Currency::parse(&input.currency)?;
                    let mut lines = Vec::with_capacity(input.lines.len());
                    for item in input.lines {
                        let price = Money::from_decimal(
                            autoxide_kernel::decimal::parse(&item.unit_price)?,
                            currency,
                            Rounding::HalfUp,
                        )?;
                        let quantity = match item.quantity {
                            Some(quantity) => autoxide_kernel::decimal::parse(&quantity)?,
                            None => rust_decimal::Decimal::ONE,
                        };
                        let mut line = NewDocumentLine::new(item.description, quantity, price);
                        line.quantity = Some(DecimalString(quantity));
                        line.unit = item.unit;
                        line.tax_category = item.tax_category;
                        lines.push(line);
                    }
                    let buyer = Buyer {
                        kind: input.buyer_kind.unwrap_or_else(|| "consumer".to_owned()),
                        name: input.buyer_name,
                        tax_code: input.buyer_tax_code,
                        address: input.buyer_address,
                        email: input.buyer_email,
                    };
                    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
                    let draft = NewDocument::new(org, input.kind, currency, buyer, lines, "invoices", input.reference);
                    let found = service.create_manual(&tenant, &principal, draft).await?.into_inner();
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(WRITE),
        Tool::new(
            "issue",
            "Issue a draft: gives it the next number of its series (no gaps), books the sale and sends \
             the e-invoice. Issuing again returns the same number.",
            move |tenant: TenantContext, principal: Principal, input: DocumentId| {
                let service = issue.clone();
                async move { service.issue_manual(&tenant, &principal, input.document_id).await }
            },
        )
        .requires(ISSUE),
    ]
}
