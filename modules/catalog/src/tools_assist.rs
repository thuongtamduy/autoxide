//! Tool MCP cho nhập CSV và bản nháp AI — cùng permission và service với REST (`/imports`,
//! `/products/{id}/suggestions`), không có logic riêng (luật kiến trúc 8).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::suggestions::SuggestionResponse;
use crate::http::transfers::TransferResponse;
use crate::permissions::{IMPORT, PRODUCTS_WRITE};
use crate::service::CatalogService;

/// A CSV file to import.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogStartImportInput)]
pub(crate) struct StartImportInput {
    /// The whole CSV file (first line: column names `sku`, `name`, `locale`, `short_description`,
    /// `description`, `kind`, `tax_category`, `barcode`, `weight_grams`).
    pub csv: String,
    /// Unit that owns the new products; leave out when you work in one unit only.
    pub org_id: Option<Uuid>,
}

/// An import to look at.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogImportId)]
pub(crate) struct ImportId {
    /// Id of the import, from `start_import`.
    pub id: Uuid,
}

/// A product to draft texts for.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogSuggestCopyInput)]
pub(crate) struct SuggestCopyInput {
    /// Id of the product.
    pub product_id: Uuid,
    /// Language of the draft, e.g. `vi`.
    pub locale: String,
}

pub(crate) fn all(service: &Arc<CatalogService>) -> Vec<Tool> {
    let (import, status, copy) = (service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "start_import",
            "Import products from a CSV file in the background: a known SKU updates that variant \
             and its text, a new SKU creates a draft product. Nothing is put on sale. Follow with \
             `import_status`.",
            move |tenant: TenantContext, principal: Principal, input: StartImportInput| {
                let service = import.clone();
                async move {
                    let body = bytes::Bytes::from(input.csv);
                    let row = service.start_import(&tenant, &principal, input.org_id, body).await?;
                    Ok::<_, Error>(TransferResponse::from(row))
                }
            },
        )
        // Quyền ghi sản phẩm được kiểm cho từng dòng khi job chạy (như REST).
        .requires(IMPORT),
        Tool::new(
            "import_status",
            "Status of an import you started, with the counts and the errors by row.",
            move |tenant: TenantContext, principal: Principal, input: ImportId| {
                let service = status.clone();
                async move {
                    let row = service.get_transfer(&tenant, &principal, "import", input.id).await?;
                    Ok::<_, Error>(TransferResponse::from(row))
                }
            },
        )
        // Không `read_only`: `catalog.import` là quyền mức ghi.
        .requires(IMPORT),
        Tool::new(
            "suggest_copy",
            "Ask AI for a draft short description, description and SEO texts of a product. The \
             draft changes nothing until a person applies it (`POST \
             /api/v1/catalog/suggestions/{id}/apply`).",
            move |tenant: TenantContext, principal: Principal, input: SuggestCopyInput| {
                let service = copy.clone();
                async move {
                    let row = service
                        .suggest_copy(&tenant, &principal, input.product_id, &input.locale)
                        .await?;
                    Ok::<_, Error>(SuggestionResponse::from(row))
                }
            },
        )
        .requires(PRODUCTS_WRITE),
    ]
}
