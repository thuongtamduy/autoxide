//! Endpoint REST của module `analytics`, dưới `/api/v1/analytics`. Handler chỉ: lấy dữ liệu vào →
//! gọi service → trả envelope. Báo cáo chỉ đọc bảng gộp; quyền kiểm tại nút của website.

use autoxide_kernel::time::Date;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest, page_of};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::errors;
use crate::ingest::{IncomingEvent, Ingested, Visitor};
use crate::permissions::ANALYTICS_READ;
use crate::reports::{Dimension, FunnelReport, Overview, Ranked};
use crate::service::{AnalyticsService, now};
use crate::shared::valid_kind;

pub(crate) fn routes(service: Arc<AnalyticsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(collect))
        .routes(routes!(event_types))
        .routes(routes!(list_funnels))
        .routes(routes!(overview))
        .routes(routes!(pages))
        .routes(routes!(sources))
        .routes(routes!(campaigns))
        .routes(routes!(types))
        .routes(routes!(objects))
        .routes(routes!(funnel))
        .with_state(service)
}

/// A batch of events from a page.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventBatch {
    /// 1 to 20 events. Events of unknown types, with undeclared or wrongly typed `props`, or
    /// otherwise malformed are dropped one by one; the batch is never refused for them.
    #[validate(length(min = 1, max = 20))]
    #[schema(value_type = Vec<IncomingEvent>, min_items = 1, max_items = 20)]
    events: Vec<Value>,
}

/// Send visit events (public).
///
/// No sign-in needed; call it from the pages of the website, e.g. with `fetch(url, {method:
/// "POST", keepalive: true, headers: {"content-type": "application/json"}, body})`, which still
/// goes out while the page closes. No cookie is set or read. The website is the one of the domain
/// called (or `?site=`). Batches from known bots, from pages whose `Origin` (or `Referer`) is not
/// a domain of this website, or over the per-network limit are dropped silently. Signed-in users
/// who consented to `analytics.personal` have their events linked to their account; nobody else.
#[utoipa::path(
    post,
    path = "/public/events",
    extensions(("x-public" = json!(true))),
    operation_id = "collect_analytics_events",
    security((), ("bearer" = [])),
    params(SiteQuery),
    request_body = EventBatch,
    responses(
        (status = 202, description = "The batch was taken; `accepted` events were stored and `dropped` were not.", body = Data<Ingested>),
        (status = 400, description = "No tenant could be told for the request; code `tenant_required`.", body = Problem),
        (status = 404, description = "There is no such website; code `site_not_found`.", body = Problem),
        (status = 415, description = "The body is not `application/json`; code `unsupported_media_type`.", body = Problem),
        (status = 422, description = "The batch is empty or holds more than 20 events; see `errors`.", body = Problem),
    )
)]
async fn collect(
    State(service): State<Arc<AnalyticsService>>,
    tenant: TenantContext,
    site: Site,
    client: ClientInfo,
    principal: Option<Principal>,
    headers: HeaderMap,
    ValidJson(batch): ValidJson<EventBatch>,
) -> Result<(StatusCode, Data<Ingested>), Error> {
    let host = headers.get(header::HOST).and_then(|value| value.to_str().ok());
    let visitor =
        Visitor { client: &client, host, user_id: principal.as_ref().map(Principal::user_id) };
    let ingested = service.ingest(&tenant, &site, visitor, batch.events, now()).await?;
    Ok((StatusCode::ACCEPTED, Data { data: ingested }))
}

/// A prop an event type takes.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PropInfo {
    name: String,
    /// `text` (up to 100 characters), `number` or `boolean`.
    kind: String,
}

/// An event type.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct EventTypeInfo {
    /// The `type` to send, e.g. `catalog.product_viewed`.
    key: String,
    description: String,
    /// `browser` (pages send it) or `server` (recorded by the server from what happened; pages
    /// cannot send it).
    source: String,
    /// When set, events must carry `object` = `<object_kind>:<id>`.
    object_kind: Option<String>,
    props: Vec<PropInfo>,
}

/// List the event types (public).
///
/// No sign-in needed. Every event type the installed modules declare: what pages may send and
/// which props each type takes. By key unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/event-types",
    extensions(("x-public" = json!(true))),
    operation_id = "list_analytics_event_types",
    security((), ("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the event types, by key.", body = Page<EventTypeInfo>),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn event_types(
    State(service): State<Arc<AnalyticsService>>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<EventTypeInfo>, Error> {
    let page = PageRequest::try_from(page)?;
    // Sổ loại event do module khai trong code: phân trang trong bộ nhớ (D-20261125-list-pages mục 4).
    let types: Vec<EventTypeInfo> = service
        .catalog
        .events()
        .map(|event| EventTypeInfo {
            key: event.key.to_owned(),
            description: event.description.to_owned(),
            source: if event.from_browser() { "browser" } else { "server" }.to_owned(),
            object_kind: event.object_kind.map(str::to_owned),
            props: event
                .props
                .iter()
                .map(|prop| PropInfo {
                    name: prop.name.to_owned(),
                    kind: prop.kind.as_str().to_owned(),
                })
                .collect(),
        })
        .collect();
    page_of(types, |event| event.key.clone(), &page)
}

/// A funnel a module declares.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct FunnelInfo {
    key: String,
    description: String,
    /// Event types of the steps, in order.
    steps: Vec<String>,
}

/// List the funnels.
///
/// Requires `analytics.read`. Funnels are declared by the installed modules. By key unless
/// `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/funnels",
    extensions(("x-permissions" = json!(["analytics.read"]))),
    operation_id = "list_analytics_funnels",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the funnels, by key.", body = Page<FunnelInfo>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `analytics.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_funnels(
    State(service): State<Arc<AnalyticsService>>,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<FunnelInfo>, Error> {
    principal.require(&ANALYTICS_READ)?;
    let page = PageRequest::try_from(page)?;
    // Phễu do module khai trong code: phân trang trong bộ nhớ (D-20261125-list-pages mục 4).
    let funnels: Vec<FunnelInfo> = service
        .catalog
        .funnels()
        .map(|funnel| FunnelInfo {
            key: funnel.key.to_owned(),
            description: funnel.description.to_owned(),
            steps: funnel.steps.iter().map(|step| (*step).to_owned()).collect(),
        })
        .collect();
    page_of(funnels, |funnel| funnel.key.clone(), &page)
}

/// Which website and days.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RangeQuery {
    /// The website (organization unit); omitted: the website of the whole tenant. You need
    /// `analytics.read` at that unit.
    pub site: Option<Uuid>,
    /// First day (UTC, `YYYY-MM-DD`); omitted: 29 days before `to`.
    #[param(value_type = Option<String>, format = Date)]
    pub from: Option<Date>,
    /// Last day (UTC, `YYYY-MM-DD`); omitted: today. At most 400 days from `from`.
    #[param(value_type = Option<String>, format = Date)]
    pub to: Option<Date>,
}

/// Get the visit overview of a website.
///
/// Requires `analytics.read` at the website's unit. Visitors, page views, events, bounce rate, the
/// top sources and one entry per day, from the hourly daily totals (today's numbers lag by up to
/// an hour).
#[utoipa::path(
    get,
    path = "/overview",
    extensions(("x-permissions" = json!(["analytics.read"]))),
    operation_id = "get_analytics_overview",
    security(("bearer" = [])),
    params(RangeQuery),
    responses(
        (status = 200, description = "The overview.", body = Data<Overview>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `analytics.read` at the website's unit; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "There is no such website; code `analytics.site_unknown`.", body = Problem),
        (status = 422, description = "The range is longer than 400 days (`analytics.range_too_long`) or reversed (`analytics.range_reversed`).", body = Problem),
    )
)]
async fn overview(
    State(service): State<Arc<AnalyticsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RangeQuery>,
) -> Result<Data<Overview>, Error> {
    let range = service.range(&tenant, &principal, query.site, query.from, query.to).await?;
    Ok(Data { data: service.overview(&tenant, &range).await? })
}

// `analytics.read` (ở bất kỳ nút nào; nút của website do `range` kiểm) trước khi đọc tham số trang, để người
// không có quyền nhận 403 chứ không phải 422 (D-20261126-list-pages-cluster-2).
async fn ranked(
    service: &AnalyticsService,
    tenant: &TenantContext,
    principal: &Principal,
    query: RangeQuery,
    page: NaturalOrderPageQuery,
    dimension: Dimension,
    prefix: Option<&str>,
) -> Result<Page<Ranked>, Error> {
    principal.require(&ANALYTICS_READ)?;
    let page = PageRequest::try_from(page)?;
    let range = service.range(tenant, principal, query.site, query.from, query.to).await?;
    service.ranking_page(tenant, &range, dimension, prefix, &page).await
}

/// List the most viewed pages.
///
/// Requires `analytics.read` at the website's unit. Page paths by page views, most first (then by
/// path; `order=desc` reverses it); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/pages",
    extensions(("x-permissions" = json!(["analytics.read"]))),
    operation_id = "list_analytics_pages",
    security(("bearer" = [])),
    params(RangeQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the pages, most viewed first.", body = Page<Ranked>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `analytics.read` at the website's unit.", body = Problem),
        (status = 404, description = "There is no such website; code `analytics.site_unknown`.", body = Problem),
        (status = 422, description = "The range is too long or reversed, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn pages(
    State(service): State<Arc<AnalyticsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RangeQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<Ranked>, Error> {
    ranked(&service, &tenant, &principal, query, page, Dimension::Page, None).await
}

/// List the traffic sources.
///
/// Requires `analytics.read` at the website's unit. Where visits came from, by page views that
/// started outside the website: the `utm_source`, else the referring site (without `www.`), else
/// `direct`. Most first (then by name; `order=desc` reverses it); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/sources",
    extensions(("x-permissions" = json!(["analytics.read"]))),
    operation_id = "list_analytics_sources",
    security(("bearer" = [])),
    params(RangeQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the sources, most page views first.", body = Page<Ranked>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `analytics.read` at the website's unit.", body = Problem),
        (status = 404, description = "There is no such website; code `analytics.site_unknown`.", body = Problem),
        (status = 422, description = "The range is too long or reversed, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn sources(
    State(service): State<Arc<AnalyticsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RangeQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<Ranked>, Error> {
    ranked(&service, &tenant, &principal, query, page, Dimension::Source, None).await
}

/// List the UTM campaigns.
///
/// Requires `analytics.read` at the website's unit. `utm_campaign` values (lowercased) by page
/// views that started outside the website. Most first (then by name; `order=desc` reverses it);
/// paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/campaigns",
    extensions(("x-permissions" = json!(["analytics.read"]))),
    operation_id = "list_analytics_campaigns",
    security(("bearer" = [])),
    params(RangeQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the campaigns, most page views first.", body = Page<Ranked>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `analytics.read` at the website's unit.", body = Problem),
        (status = 404, description = "There is no such website; code `analytics.site_unknown`.", body = Problem),
        (status = 422, description = "The range is too long or reversed, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn campaigns(
    State(service): State<Arc<AnalyticsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RangeQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<Ranked>, Error> {
    ranked(&service, &tenant, &principal, query, page, Dimension::Campaign, None).await
}

/// List the event types by count.
///
/// Requires `analytics.read` at the website's unit. Every event type seen, most events first (then
/// by key; `order=desc` reverses it); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/types",
    extensions(("x-permissions" = json!(["analytics.read"]))),
    operation_id = "list_analytics_type_counts",
    security(("bearer" = [])),
    params(RangeQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the event types, most events first.", body = Page<Ranked>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `analytics.read` at the website's unit.", body = Problem),
        (status = 404, description = "There is no such website; code `analytics.site_unknown`.", body = Problem),
        (status = 422, description = "The range is too long or reversed, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn types(
    State(service): State<Arc<AnalyticsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RangeQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<Ranked>, Error> {
    ranked(&service, &tenant, &principal, query, page, Dimension::Type, None).await
}

/// Which objects.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ObjectQuery {
    /// Only objects of this kind, e.g. `catalog.product`; omitted: every kind.
    #[param(example = "catalog.product")]
    pub kind: Option<String>,
}

/// List the most viewed objects.
///
/// Requires `analytics.read` at the website's unit. Objects (`<kind>:<id>`) by the events about
/// them, most first (then by object; `order=desc` reverses it); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/objects",
    extensions(("x-permissions" = json!(["analytics.read"]))),
    operation_id = "list_analytics_objects",
    security(("bearer" = [])),
    params(RangeQuery, ObjectQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the objects, most viewed first.", body = Page<Ranked>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `analytics.read` at the website's unit.", body = Problem),
        (status = 404, description = "There is no such website; code `analytics.site_unknown`.", body = Problem),
        (status = 422, description = "The range is too long or reversed, `kind` is malformed (`analytics.kind_invalid`), or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn objects(
    State(service): State<Arc<AnalyticsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RangeQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
    Query(object): Query<ObjectQuery>,
) -> Result<Page<Ranked>, Error> {
    let prefix = match object.kind.as_deref().map(str::trim).filter(|kind| !kind.is_empty()) {
        Some(kind) if valid_kind(kind) => Some(format!("{kind}:")),
        Some(_) => return Err(errors::kind_invalid()),
        None => None,
    };
    ranked(&service, &tenant, &principal, query, page, Dimension::Object, prefix.as_deref()).await
}

/// Get a funnel.
///
/// Requires `analytics.read` at the website's unit. For each step, the sessions with its event,
/// summed over the days. Cookieless sessions cannot be linked to what the server records (e.g. a
/// paid order), so each step is counted on its own: a later step may exceed an earlier one.
#[utoipa::path(
    get,
    path = "/funnels/{key}",
    extensions(("x-permissions" = json!(["analytics.read"]))),
    operation_id = "get_analytics_funnel",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "The funnel, e.g. `orders.checkout`."), RangeQuery),
    responses(
        (status = 200, description = "The funnel.", body = Data<FunnelReport>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `analytics.read` at the website's unit.", body = Problem),
        (status = 404, description = "No such website (`analytics.site_unknown`) or funnel (`analytics.funnel_unknown`).", body = Problem),
        (status = 422, description = "The range is too long or reversed, or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn funnel(
    State(service): State<Arc<AnalyticsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    Query(query): Query<RangeQuery>,
) -> Result<Data<FunnelReport>, Error> {
    let range = service.range(&tenant, &principal, query.site, query.from, query.to).await?;
    Ok(Data { data: service.funnel(&tenant, &range, &key).await? })
}
