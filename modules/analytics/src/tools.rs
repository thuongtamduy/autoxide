//! Tool MCP của module `analytics` — chỉ đọc. Mỗi tool: một kiểu đầu vào, permission như endpoint
//! REST tương ứng, thân là lời gọi service (luật kiến trúc 8).

use autoxide_kernel::time::Date;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest};
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::errors;
use crate::permissions::ANALYTICS_READ;
use crate::reports::Dimension;
use crate::service::AnalyticsService;
use crate::shared::valid_kind;

/// A website and a range of days.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RangeInput {
    /// The website (organization unit); omitted: the website of the whole tenant.
    site: Option<Uuid>,
    /// First day (UTC, `YYYY-MM-DD`); omitted: 29 days before `to`.
    #[schema(value_type = Option<String>, format = Date)]
    from: Option<Date>,
    /// Last day (UTC, `YYYY-MM-DD`); omitted: today. At most 400 days from `from`.
    #[schema(value_type = Option<String>, format = Date)]
    to: Option<Date>,
}

/// A ranking over a website and a range of days.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TopInput {
    site: Option<Uuid>,
    #[schema(value_type = Option<String>, format = Date)]
    from: Option<Date>,
    #[schema(value_type = Option<String>, format = Date)]
    to: Option<Date>,
    /// For objects only: just this kind, e.g. `catalog.product`.
    kind: Option<String>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `asc` (most first, the default) or `desc` (fewest first).
    order: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// A funnel over a website and a range of days.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FunnelInput {
    /// The funnel, e.g. `orders.checkout`; see `GET /api/v1/analytics/funnels`.
    key: String,
    site: Option<Uuid>,
    #[schema(value_type = Option<String>, format = Date)]
    from: Option<Date>,
    #[schema(value_type = Option<String>, format = Date)]
    to: Option<Date>,
}

async fn top(
    service: &AnalyticsService,
    tenant: &TenantContext,
    principal: &Principal,
    input: TopInput,
    dimension: Dimension,
) -> Result<Page<crate::reports::Ranked>, Error> {
    // Cùng kiểu tham số trang và cùng hàm với REST (`NaturalOrderPageQuery`: mặc định nhiều nhất trước).
    let page = PageRequest::try_from(
        NaturalOrderPageQuery::new(
            input.limit.map(|limit| limit.to_string()),
            input.cursor,
            input.order,
        )
        .with_count(input.count.unwrap_or(false)),
    )?;
    let prefix = match input.kind.as_deref().map(str::trim).filter(|kind| !kind.is_empty()) {
        Some(kind) if valid_kind(kind) => Some(format!("{kind}:")),
        Some(_) => return Err(errors::kind_invalid()),
        None => None,
    };
    let range = service.range(tenant, principal, input.site, input.from, input.to).await?;
    service.ranking_page(tenant, &range, dimension, prefix.as_deref(), &page).await
}

pub(crate) fn all(service: &Arc<AnalyticsService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (overview, pages, objects, funnel) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "overview",
            "Visitors, page views, bounce rate, top sources and daily counts of a website over a \
             range of days (from the daily totals).",
            move |tenant: TenantContext, principal: Principal, input: RangeInput| {
                let service = overview.clone();
                async move {
                    let range = service
                        .range(&tenant, &principal, input.site, input.from, input.to)
                        .await?;
                    service.overview(&tenant, &range).await
                }
            },
        )
        .requires(ANALYTICS_READ)
        .read_only(),
        Tool::new(
            "top_pages",
            "The most viewed pages of a website over a range of days, most first; the same pages as \
             `GET /api/v1/analytics/pages`; follow `pagination.next_cursor`.",
            move |tenant: TenantContext, principal: Principal, input: TopInput| {
                let service = pages.clone();
                async move { top(&service, &tenant, &principal, input, Dimension::Page).await }
            },
        )
        .requires(ANALYTICS_READ)
        .read_only(),
        Tool::new(
            "top_objects",
            "The most viewed objects (`<kind>:<id>`, e.g. products) of a website over a range of \
             days, most first; `kind` keeps one kind; follow `pagination.next_cursor`.",
            move |tenant: TenantContext, principal: Principal, input: TopInput| {
                let service = objects.clone();
                async move { top(&service, &tenant, &principal, input, Dimension::Object).await }
            },
        )
        .requires(ANALYTICS_READ)
        .read_only(),
        Tool::new(
            "funnel",
            "Sessions reaching each step of a declared funnel on a website over a range of days; \
             steps are counted on their own.",
            move |tenant: TenantContext, principal: Principal, input: FunnelInput| {
                let service = funnel.clone();
                async move {
                    let range = service
                        .range(&tenant, &principal, input.site, input.from, input.to)
                        .await?;
                    service.funnel(&tenant, &range, &input.key).await
                }
            },
        )
        .requires(ANALYTICS_READ)
        .read_only(),
    ]
}
