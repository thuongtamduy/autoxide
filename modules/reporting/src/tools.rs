//! Tool MCP của module `reporting`: cùng service, cùng permission, cùng phạm vi thấy với REST
//! (luật kiến trúc 8).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use validator::Validate;

use crate::http::{FilterQuery, SummaryOptions, SummaryResponse, checked, fact_page, kind_list};
use crate::permissions::REPORTING_READ;
use crate::service::Reporting;

/// No input.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Empty {}

/// Which report figures to sum, and how.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SummaryInput {
    /// The fact kind, e.g. `invoices.issued_total` (see `reporting.kinds`).
    kind: String,
    /// First day (inclusive), `YYYY-MM-DD`; defaults to 29 days before `to`.
    from: Option<String>,
    /// Last day (inclusive), `YYYY-MM-DD`; defaults to today (UTC).
    to: Option<String>,
    /// `day` (default, at most 366 days), `week`, `month`, `tenant` or `org`.
    group_by: Option<String>,
    /// Also convert every sum to this ISO 4217 currency with your tenant's exchange rates.
    currency: Option<String>,
    /// Only these tenants of your customer tree, comma-separated ids.
    tenant_ids: Option<String>,
    /// Only these organization units, comma-separated ids.
    org_ids: Option<String>,
}

/// Which report facts to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FactsInput {
    /// The fact kind, e.g. `invoices.issued_total`.
    kind: Option<String>,
    /// First day (inclusive), `YYYY-MM-DD`.
    from: Option<String>,
    /// Last day (inclusive), `YYYY-MM-DD`.
    to: Option<String>,
    /// Only these tenants of your customer tree, comma-separated ids.
    tenant_ids: Option<String>,
    /// Only these organization units, comma-separated ids.
    org_ids: Option<String>,
    /// Comma-separated `name:value` pairs, e.g. `kind:vat_invoice` (needs `kind`).
    dimensions: Option<String>,
    /// Maximum number of facts, 1 to 200 (default 50).
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, default) or `asc`.
    order: Option<String>,
}

pub(crate) fn all(service: &Arc<Reporting>) -> Vec<Tool> {
    let (kinds, summary, facts) = (service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "kinds",
            "List the kinds of report figures modules declare (kind, unit, dimensions).",
            move |_tenant: TenantContext, principal: Principal, _input: Empty| {
                let service = kinds.clone();
                async move {
                    principal.require(&REPORTING_READ)?;
                    Ok::<_, Error>(kind_list(&service))
                }
            },
        )
        .requires(REPORTING_READ)
        .read_only(),
        Tool::new(
            "summary",
            "Sum a report figure over a period, per day, week, month, tenant or unit, for your \
             tenant and the tenants below it (as far as your scope reaches). Money is summed per \
             currency; ask `currency` to also get a converted total.",
            move |tenant: TenantContext, principal: Principal, input: SummaryInput| {
                let service = summary.clone();
                async move {
                    let filter = FilterQuery {
                        kind: Some(input.kind),
                        from: input.from,
                        to: input.to,
                        tenant_ids: input.tenant_ids,
                        org_ids: input.org_ids,
                        dimensions: None,
                    };
                    let options =
                        SummaryOptions { group_by: input.group_by, currency: input.currency };
                    let mut problems = Vec::new();
                    let filter = filter.read(&mut problems);
                    let query = checked(options.read(filter, &mut problems), problems)?;
                    let answer = service.summary(&tenant, &principal, &query).await?;
                    Ok::<_, Error>(SummaryResponse::from(answer))
                }
            },
        )
        .requires(REPORTING_READ)
        .read_only(),
        Tool::new(
            "facts",
            "List report facts (newest first) with the same visibility as `reporting.summary`; \
             follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: FactsInput| {
                let service = facts.clone();
                async move {
                    let filter = FilterQuery {
                        kind: input.kind,
                        from: input.from,
                        to: input.to,
                        tenant_ids: input.tenant_ids,
                        org_ids: input.org_ids,
                        dimensions: input.dimensions,
                    };
                    let mut problems = Vec::new();
                    let filter = checked(filter.read(&mut problems), problems)?;
                    let page = PageInput::new(input.limit, input.cursor, input.order).page()?;
                    fact_page(service.facts(&tenant, &principal, &filter, &page).await?)
                }
            },
        )
        .requires(REPORTING_READ)
        .read_only(),
    ]
}
