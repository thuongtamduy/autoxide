//! Endpoint REST của module `webhooks`, dưới `/api/v1/webhooks`. Mọi endpoint cần
//! permission `webhooks.manage`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::permissions::WEBHOOKS_MANAGE;
use crate::service::{Delivery, Endpoint, EndpointChanges, NewEndpoint, WebhookService};

pub(crate) fn routes(service: Arc<WebhookService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_event_types))
        .routes(routes!(list_endpoints, create_endpoint))
        .routes(routes!(get_endpoint, update_endpoint, delete_endpoint))
        .routes(routes!(rotate_secret))
        .routes(routes!(list_deliveries))
        .routes(routes!(list_all_deliveries))
        .routes(routes!(redeliver))
        .with_state(service)
}

/// `Cache-Control: no-store` cho response chứa secret chỉ trả một lần: proxy không giữ, lớp
/// idempotency không lưu vào database (D-149, D-157).
type NoStore = [(axum::http::HeaderName, &'static str); 1];
const NO_STORE: NoStore = [(axum::http::header::CACHE_CONTROL, "no-store")];

/// A kind of event that endpoints can subscribe to.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct EventTypeResponse {
    #[schema(example = "auth.user_created")]
    name: String,
    description: String,
}

/// A webhook endpoint: an address that receives events. Its secret is never listed.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct EndpointResponse {
    id: Uuid,
    #[schema(example = "https://example.com/hooks/autoxide")]
    url: String,
    #[schema(example = "Sync users to the CRM")]
    description: String,
    /// Event names it receives; `*` means every event.
    #[schema(example = json!(["auth.user_created", "auth.user_deleted"]))]
    events: Vec<String>,
    /// When it was turned off; `null` while it receives events.
    disabled_at: Option<DateTime<Utc>>,
    /// The newest delivery to this endpoint; `null` when nothing was sent yet. Only in the list
    /// and when reading one endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_delivery: Option<Option<LastDelivery>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// The newest delivery to an endpoint.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct LastDelivery {
    id: Uuid,
    #[schema(example = "auth.user_created")]
    event_name: String,
    /// `pending` (sending or waiting for a retry), `succeeded` or `failed` (gave up) — the same values
    /// as a delivery's `status`.
    #[schema(example = "succeeded")]
    status: String,
    /// HTTP status the endpoint answered with, on the last attempt; `null` when it did not answer.
    response_status: Option<i32>,
    /// When it was last tried; `null` while it waits for its first attempt.
    last_attempt_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<crate::entity::delivery::Model> for LastDelivery {
    fn from(row: crate::entity::delivery::Model) -> Self {
        LastDelivery {
            id: row.id,
            event_name: row.event_name,
            status: row.status,
            response_status: row.response_status,
            last_attempt_at: row.last_attempt_at,
            created_at: row.created_at,
        }
    }
}

/// Which endpoints to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EndpointFilter {
    /// `active`: only endpoints that receive events; `disabled`: only turned-off ones.
    #[param(inline)]
    status: Option<EndpointStatus>,
}

/// Whether an endpoint receives events.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EndpointStatus {
    Active,
    Disabled,
}

impl From<Endpoint> for EndpointResponse {
    fn from(endpoint: Endpoint) -> Self {
        EndpointResponse {
            id: endpoint.id,
            url: endpoint.url,
            description: endpoint.description,
            events: endpoint.events,
            disabled_at: endpoint.disabled_at,
            last_delivery: None,
            created_at: endpoint.created_at,
            updated_at: endpoint.updated_at,
        }
    }
}

/// A new endpoint and its signing secret, shown only once.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct CreatedEndpointResponse {
    /// Verify the `webhook-signature` header of each request with this secret (Standard
    /// Webhooks). Store it now: it cannot be shown again, only replaced.
    #[schema(example = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw")]
    secret: String,
    #[serde(flatten)]
    endpoint: EndpointResponse,
}

/// A new signing secret, shown only once.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct SecretResponse {
    #[schema(example = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw")]
    secret: String,
}

/// A new endpoint.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateEndpointRequest {
    /// Where to send events: an `https://` address, up to 2048 characters.
    #[schema(example = "https://example.com/hooks/autoxide")]
    url: String,
    /// What the endpoint is for, up to 200 characters.
    #[serde(default)]
    description: String,
    /// Event names to receive (see `GET /api/v1/webhooks/events`), or `["*"]` for every
    /// event.
    #[schema(example = json!(["auth.user_created"]))]
    events: Vec<String>,
}

/// Changes to an endpoint. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateEndpointRequest {
    url: Option<String>,
    description: Option<String>,
    events: Option<Vec<String>>,
    /// `true` stops sending to it (pending retries are dropped); `false` starts again.
    disabled: Option<bool>,
}

/// One attempt to deliver one event to one endpoint, with its retries.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct DeliveryResponse {
    /// Also the `webhook-id` header: the same on every retry, to drop duplicates.
    id: Uuid,
    /// Id of the event (`data.id` in the body).
    event_id: Uuid,
    #[schema(example = "auth.user_created")]
    event_name: String,
    /// `pending` (sending or waiting for a retry), `succeeded` or `failed` (gave up).
    #[schema(example = "succeeded")]
    status: String,
    attempts: i32,
    /// HTTP status of the last answer; `null` when there was none.
    #[schema(example = 200)]
    response_status: Option<i32>,
    /// Why the last attempt failed.
    last_error: Option<String>,
    created_at: DateTime<Utc>,
    last_attempt_at: Option<DateTime<Utc>>,
    delivered_at: Option<DateTime<Utc>>,
}

impl From<Delivery> for DeliveryResponse {
    fn from(row: Delivery) -> Self {
        DeliveryResponse {
            id: row.id,
            event_id: row.event_id,
            event_name: row.event_name,
            status: row.status,
            attempts: row.attempts,
            response_status: row.response_status,
            last_error: row.last_error,
            created_at: row.created_at,
            last_attempt_at: row.last_attempt_at,
            delivered_at: row.delivered_at,
        }
    }
}

/// List the kinds of events endpoints can receive.
///
/// Requires `webhooks.manage`. Sorted by name; descriptions follow `Accept-Language`.
#[utoipa::path(
    get,
    path = "/events",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "list_webhook_event_types",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every event kind.", body = Data<Vec<EventTypeResponse>>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_event_types(
    State(service): State<Arc<WebhookService>>,
    principal: Principal,
    language: autoxide_kernel::i18n::Language,
) -> Result<Data<Vec<EventTypeResponse>>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let data = service
        .event_catalog()
        .iter()
        .map(|event| EventTypeResponse {
            name: event.name().to_owned(),
            description: service.describe(event.description(), language),
        })
        .collect();
    Ok(Data { data })
}

/// List webhook endpoints.
///
/// Requires `webhooks.manage`. Newest first unless `order=asc`. Each endpoint carries its
/// newest delivery (`last_delivery`); `status` keeps only active or disabled endpoints.
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "list_webhook_endpoints",
    security(("bearer" = [])),
    params(EndpointFilter, PageQuery),
    responses(
        (status = 200, description = "One page of endpoints.", body = Page<EndpointResponse>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_endpoints(
    State(service): State<Arc<WebhookService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<EndpointFilter>,
    Query(query): Query<PageQuery>,
) -> Result<Page<EndpointResponse>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let active = filter.status.map(|status| matches!(status, EndpointStatus::Active));
    let page = service.list(&tenant, active, &PageRequest::try_from(query)?).await?;
    let ids: Vec<Uuid> = page.data.iter().map(|endpoint| endpoint.id).collect();
    let mut last = service.last_deliveries(&tenant, &ids).await?;
    Ok(page.map(|endpoint| {
        let delivery = last.remove(&endpoint.id).map(LastDelivery::from);
        EndpointResponse { last_delivery: Some(delivery), ..EndpointResponse::from(endpoint) }
    }))
}

/// Create a webhook endpoint.
///
/// Requires `webhooks.manage`. From now on, every event it subscribes to is sent to
/// `url` as a signed `POST` (Standard Webhooks headers `webhook-id`,
/// `webhook-timestamp`, `webhook-signature`); answer with any 2xx within 10 seconds
/// (`AUTOXIDE_WEBHOOKS_TIMEOUT_SECONDS`).
/// Other answers are retried with growing delays for about two hours. The response
/// carries the signing secret, shown only once.
#[utoipa::path(
    post,
    path = "/",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "create_webhook_endpoint",
    security(("bearer" = [])),
    request_body = CreateEndpointRequest,
    responses(
        (status = 201, description = "Created; store `secret` now.", body = Data<CreatedEndpointResponse>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid — e.g. the address is not `https://` or an event name is unknown; `errors` lists every problem.", body = Problem),
    )
)]
async fn create_endpoint(
    State(service): State<Arc<WebhookService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateEndpointRequest>,
) -> Result<(NoStore, Created<CreatedEndpointResponse>), Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let input =
        NewEndpoint { url: input.url, description: input.description, events: input.events };
    let created = service.create(&tenant, Actor::from(&principal), input).await?;
    let path = format!("/webhooks/{}", created.endpoint.id);
    let data =
        CreatedEndpointResponse { secret: created.secret, endpoint: created.endpoint.into() };
    Ok((NO_STORE, Created::new(path, data)))
}

/// Get a webhook endpoint.
///
/// Requires `webhooks.manage`.
#[utoipa::path(
    get,
    path = "/{id}",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "get_webhook_endpoint",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the endpoint.")),
    responses(
        (status = 200, description = "The endpoint.", body = Data<EndpointResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No endpoint has this id; code `webhooks.endpoint_not_found`.", body = Problem),
    )
)]
async fn get_endpoint(
    State(service): State<Arc<WebhookService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<EndpointResponse>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let endpoint = service.get(&tenant, id).await?;
    let delivery =
        service.last_deliveries(&tenant, &[id]).await?.remove(&id).map(LastDelivery::from);
    let version = endpoint.updated_at;
    let data =
        EndpointResponse { last_delivery: Some(delivery), ..EndpointResponse::from(endpoint) };
    Ok(Tagged { version, data })
}

/// Change a webhook endpoint, or turn it off and on.
///
/// Requires `webhooks.manage`.
#[utoipa::path(
    patch,
    path = "/{id}",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "update_webhook_endpoint",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the endpoint."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = UpdateEndpointRequest,
    responses(
        (status = 200, description = "The changed endpoint.", body = Data<EndpointResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No endpoint has this id; code `webhooks.endpoint_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid — e.g. the address is not `https://` or an event name is unknown; `errors` lists every problem.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it. Get it again and retry; code `precondition_failed`.", body = Problem),
    )
)]
async fn update_endpoint(
    State(service): State<Arc<WebhookService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateEndpointRequest>,
) -> Result<Tagged<EndpointResponse>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let changes = EndpointChanges {
        url: input.url,
        description: input.description,
        events: input.events,
        disabled: input.disabled,
    };
    let expected = if_match.expected()?;
    let endpoint = service.update(&tenant, Actor::from(&principal), id, changes, expected).await?;
    Ok(Tagged { version: endpoint.updated_at, data: endpoint.into() })
}

/// Delete a webhook endpoint and its delivery history.
///
/// Requires `webhooks.manage`. Pending retries are dropped.
#[utoipa::path(
    delete,
    path = "/{id}",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "delete_webhook_endpoint",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the endpoint.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No endpoint has this id; code `webhooks.endpoint_not_found`.", body = Problem),
    )
)]
async fn delete_endpoint(
    State(service): State<Arc<WebhookService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    service.delete(&tenant, Actor::from(&principal), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Replace the signing secret of an endpoint.
///
/// Requires `webhooks.manage`. The old secret stops working at once: update the
/// receiver right after. The new secret is shown only once.
#[utoipa::path(
    post,
    path = "/{id}/secret",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "rotate_webhook_secret",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the endpoint.")),
    responses(
        (status = 200, description = "The new secret.", body = Data<SecretResponse>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No endpoint has this id; code `webhooks.endpoint_not_found`.", body = Problem),
    )
)]
async fn rotate_secret(
    State(service): State<Arc<WebhookService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<(NoStore, Data<SecretResponse>), Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let secret = service.rotate_secret(&tenant, Actor::from(&principal), id).await?;
    Ok((NO_STORE, Data { data: SecretResponse { secret } }))
}

/// List the deliveries of an endpoint, to see what was sent and what failed.
///
/// Requires `webhooks.manage`. Newest first unless `order=asc`. Kept for 30 days.
#[utoipa::path(
    get,
    path = "/{id}/deliveries",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "list_webhook_deliveries",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the endpoint."), PageQuery),
    responses(
        (status = 200, description = "One page of deliveries.", body = Page<DeliveryResponse>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No endpoint has this id; code `webhooks.endpoint_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_deliveries(
    State(service): State<Arc<WebhookService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Result<Page<DeliveryResponse>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let page = service.deliveries(&tenant, id, &PageRequest::try_from(query)?).await?;
    Ok(page.map(DeliveryResponse::from))
}

/// Which deliveries to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DeliveriesFilter {
    /// `pending`, `succeeded` or `failed`.
    #[param(example = "failed")]
    status: Option<String>,
    /// Only deliveries to this endpoint.
    endpoint_id: Option<Uuid>,
}

/// List the deliveries of every endpoint, e.g. the failed ones.
///
/// Requires `webhooks.manage`. Newest first unless `order=asc`. Kept for 30 days.
#[utoipa::path(
    get,
    path = "/deliveries",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "list_all_webhook_deliveries",
    security(("bearer" = [])),
    params(PageQuery, DeliveriesFilter),
    responses(
        (status = 200, description = "One page of deliveries.", body = Page<DeliveryResponse>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`status` or a paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_all_deliveries(
    State(service): State<Arc<WebhookService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
    Query(filter): Query<DeliveriesFilter>,
) -> Result<Page<DeliveryResponse>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let page = service
        .all_deliveries(
            &tenant,
            filter.status.as_deref(),
            filter.endpoint_id,
            &PageRequest::try_from(query)?,
        )
        .await?;
    Ok(page.map(DeliveryResponse::from))
}

/// Send a delivery again.
///
/// Requires `webhooks.manage`. Reopens a delivery that succeeded or failed for good and sends it
/// again with the same body and `webhook-id` (receivers can drop it if they already handled it),
/// retrying as usual. Recorded in the audit log.
#[utoipa::path(
    post,
    path = "/deliveries/{id}/redeliver",
    extensions(("x-permissions" = json!(["webhooks.manage"]))),
    operation_id = "redeliver_webhook",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the delivery.")),
    responses(
        (status = 202, description = "Queued; the delivery is `pending` again.", body = Data<DeliveryResponse>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No delivery has this id; code `webhooks.delivery_not_found`.", body = Problem),
        (status = 409, description = "The delivery is still being sent or retried; code `webhooks.delivery_in_progress`.", body = Problem),
    )
)]
async fn redeliver(
    State(service): State<Arc<WebhookService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Data<DeliveryResponse>), Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let delivery = service.redeliver(&tenant, Actor::from(&principal), id).await?;
    Ok((StatusCode::ACCEPTED, Data { data: DeliveryResponse::from(delivery) }))
}

/// Which deliveries to list.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeliveriesInput {
    /// `pending`, `succeeded` or `failed`.
    status: Option<String>,
    /// Only deliveries to this endpoint.
    endpoint_id: Option<Uuid>,
    /// 1 to 200; 50 by default.
    limit: Option<u32>,
    /// `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// The delivery to send again.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RedeliverInput {
    /// Id of the delivery.
    id: Uuid,
}

/// Tool MCP của module, gọi cùng service với REST.
pub(crate) fn tools(service: Arc<WebhookService>) -> Vec<autoxide_kernel::mcp::Tool> {
    use autoxide_kernel::mcp::Tool;
    let list = service.clone();
    vec![
        Tool::new(
            "list_deliveries",
            "List webhook deliveries of every endpoint, newest first; filter by `status` \
             (`pending`, `succeeded`, `failed`) or `endpoint_id`.",
            move |tenant: TenantContext, _: Principal, input: DeliveriesInput| {
                let service = list.clone();
                async move {
                    let page = PageRequest::try_from(PageQuery::new(
                        input.limit.map(|limit| limit.to_string()),
                        input.cursor,
                        None,
                    ))?;
                    let page = service
                        .all_deliveries(&tenant, input.status.as_deref(), input.endpoint_id, &page)
                        .await?;
                    Ok::<Page<DeliveryResponse>, Error>(page.map(DeliveryResponse::from))
                }
            },
        )
        .requires(WEBHOOKS_MANAGE)
        .read_only(),
        Tool::new(
            "redeliver",
            "Send a webhook delivery that succeeded or failed again, with the same body and \
             `webhook-id`. Refused while it is still being sent.",
            move |tenant: TenantContext, principal: Principal, input: RedeliverInput| {
                let service = service.clone();
                async move {
                    let delivery =
                        service.redeliver(&tenant, Actor::from(&principal), input.id).await?;
                    Ok::<DeliveryResponse, Error>(DeliveryResponse::from(delivery))
                }
            },
        )
        .requires(WEBHOOKS_MANAGE),
    ]
}
