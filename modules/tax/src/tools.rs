//! Tool MCP của `tax`: cùng service, cùng permission với REST; đều chỉ đọc.

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::decimal::Rounding;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest, page_of};
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::{PriceMode, TaxQuote, TaxQuoteRequest, TaxableLine};
use crate::http::rate_sort_key;
use crate::permissions::READ;
use crate::service::TaxService;

// Trường phẳng (client MCP không đọc `$ref`): dòng khai inline, số tiền là chữ thập phân theo đơn
// vị chính trong `currency` của chứng từ.
/// One line to tax.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LineInput {
    /// Your id of the line.
    line_id: String,
    /// Tax category, e.g. `vat.standard`.
    category: String,
    /// Amount in the document currency, as a decimal string, e.g. `"100000"`.
    amount: String,
    /// Quantity as a decimal string, for taxes per unit.
    quantity: Option<String>,
}

/// A document to tax.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuoteInput {
    /// Tax jurisdiction, e.g. `VN`.
    jurisdiction: String,
    /// Day the tax applies, `YYYY-MM-DD`.
    tax_point: Date,
    /// The selling unit; default the tenant's root.
    seller_org_id: Option<Uuid>,
    /// `exclusive` (amounts before taxes) or `inclusive`.
    price_mode: String,
    /// Currency code, e.g. `VND`.
    currency: String,
    /// The lines.
    #[schema(inline)]
    lines: Vec<LineInput>,
}

/// Which rates.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RatesInput {
    /// Tax jurisdiction, e.g. `VN`.
    jurisdiction: String,
    /// Day, `YYYY-MM-DD`; default today.
    on: Option<Date>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `asc` (by category, stage and start date, the default) or `desc`.
    order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// A rate in force.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct RateItem {
    category: String,
    tax_kind: String,
    basis: String,
    rate_percent: String,
    stage: i32,
    effective_from: String,
    effective_to: Option<String>,
    legal_basis: String,
}

pub(crate) fn all(service: &Arc<TaxService>) -> Vec<Tool> {
    let (quote, rates) = (service.clone(), service.clone());
    vec![
        Tool::new(
            "quote",
            "Compute the taxes of a document (per line and per rate) without saving anything. \
             Taxes are rounded once per rate on the whole document, then shared between lines.",
            move |tenant: TenantContext, principal: Principal, input: QuoteInput| {
                let service = quote.clone();
                async move {
                    let seller =
                        input.seller_org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
                    service.require_seller(&tenant, &principal, seller).await?;
                    let currency = Currency::parse(&input.currency)?;
                    let mode = match input.price_mode.as_str() {
                        "inclusive" => PriceMode::Inclusive,
                        "exclusive" => PriceMode::Exclusive,
                        _ => {
                            return Err(Error::validation(vec![FieldError::new(
                                "price_mode",
                                "invalid",
                                "must be `inclusive` or `exclusive`",
                            )]));
                        }
                    };
                    let mut lines = Vec::with_capacity(input.lines.len());
                    for item in input.lines {
                        let amount = Money::from_decimal(
                            autoxide_kernel::decimal::parse(&item.amount)?,
                            currency,
                            Rounding::HalfUp,
                        )?;
                        let mut line = TaxableLine::new(item.line_id, item.category, amount);
                        if let Some(quantity) = item.quantity {
                            line = line.with_quantity(autoxide_kernel::decimal::parse(&quantity)?);
                        }
                        lines.push(line);
                    }
                    let request = TaxQuoteRequest::new(
                        input.jurisdiction,
                        input.tax_point,
                        seller,
                        mode,
                        currency,
                        lines,
                    );
                    let found: TaxQuote = service.quote(&tenant, request).await?;
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "rates",
            "The tax rates in force on a day for a jurisdiction, with their legal basis; by category, \
             stage and start date, the same order and pages as `GET /api/v1/tax/rates`; follow \
             `pagination.next_cursor`.",
            move |_: TenantContext, principal: Principal, input: RatesInput| {
                let service = rates.clone();
                async move {
                    principal.require(&READ)?;
                    // Cùng kiểu tham số trang và cùng khoá với REST, nên cùng trang và cursor.
                    let page = PageRequest::try_from(
                        NaturalOrderPageQuery::new(
                            input.limit.map(|limit| limit.to_string()),
                            input.cursor,
                            input.order,
                        )
                        .with_count(input.count.unwrap_or(false)),
                    )?;
                    let on = input.on.unwrap_or_else(|| Timestamp::now().date_utc());
                    let day = on.format("%Y-%m-%d").to_string();
                    let tables = service.tables().await?;
                    let list: Vec<(String, RateItem)> = tables
                        .rates
                        .iter()
                        .filter(|row| row.jurisdiction == input.jurisdiction)
                        .filter(|row| {
                            row.effective_from.as_str() <= day.as_str()
                                && row.effective_to.as_deref().is_none_or(|to| day.as_str() < to)
                        })
                        .map(|row| {
                            let key = rate_sort_key(
                                &row.jurisdiction,
                                &row.category_code,
                                row.stage,
                                &row.effective_from,
                                row.id,
                            );
                            let item = RateItem {
                                category: row.category_code.clone(),
                                tax_kind: row.tax_kind.clone(),
                                basis: row.basis.clone(),
                                rate_percent: row.rate_percent.clone(),
                                stage: row.stage,
                                effective_from: row.effective_from.clone(),
                                effective_to: row.effective_to.clone(),
                                legal_basis: row.legal_basis.clone(),
                            };
                            (key, item)
                        })
                        .collect();
                    let found: Page<RateItem> =
                        page_of(list, |(key, _)| key.clone(), &page)?.map(|(_, item)| item);
                    Ok::<_, Error>(found)
                }
            },
        )
        .requires(READ)
        .read_only(),
    ]
}
