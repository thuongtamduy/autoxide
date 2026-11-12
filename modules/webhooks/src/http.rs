//! Endpoint REST của module `webhooks`, dưới `/api/v1/webhooks`. Mọi endpoint cần
//! permission `webhooks.manage`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
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
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<Endpoint> for EndpointResponse {
    fn from(endpoint: Endpoint) -> Self {
        EndpointResponse {
            id: endpoint.id,
            url: endpoint.url,
            description: endpoint.description,
            events: endpoint.events,
            disabled_at: endpoint.disabled_at,
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
/// Requires `webhooks.manage`. Sorted by name.
#[utoipa::path(
    get,
    path = "/events",
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
) -> Result<Data<Vec<EventTypeResponse>>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let data = service
        .event_catalog()
        .iter()
        .map(|event| EventTypeResponse {
            name: event.name().to_owned(),
            description: event.description().to_owned(),
        })
        .collect();
    Ok(Data { data })
}

/// List webhook endpoints.
///
/// Requires `webhooks.manage`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/",
    operation_id = "list_webhook_endpoints",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of endpoints.", body = Page<EndpointResponse>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `webhooks.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_endpoints(
    State(service): State<Arc<WebhookService>>,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<EndpointResponse>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    Ok(service.list(&PageRequest::try_from(query)?).await?.map(EndpointResponse::from))
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
    principal: Principal,
    ValidJson(input): ValidJson<CreateEndpointRequest>,
) -> Result<(NoStore, Created<CreatedEndpointResponse>), Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let input =
        NewEndpoint { url: input.url, description: input.description, events: input.events };
    let created = service.create(Actor::from(&principal), input).await?;
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
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<EndpointResponse>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let endpoint = service.get(id).await?;
    Ok(Tagged { version: endpoint.updated_at, data: endpoint.into() })
}

/// Change a webhook endpoint, or turn it off and on.
///
/// Requires `webhooks.manage`.
#[utoipa::path(
    patch,
    path = "/{id}",
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
    let endpoint = service.update(Actor::from(&principal), id, changes, expected).await?;
    Ok(Tagged { version: endpoint.updated_at, data: endpoint.into() })
}

/// Delete a webhook endpoint and its delivery history.
///
/// Requires `webhooks.manage`. Pending retries are dropped.
#[utoipa::path(
    delete,
    path = "/{id}",
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
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    service.delete(Actor::from(&principal), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Replace the signing secret of an endpoint.
///
/// Requires `webhooks.manage`. The old secret stops working at once: update the
/// receiver right after. The new secret is shown only once.
#[utoipa::path(
    post,
    path = "/{id}/secret",
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
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<(NoStore, Data<SecretResponse>), Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let secret = service.rotate_secret(Actor::from(&principal), id).await?;
    Ok((NO_STORE, Data { data: SecretResponse { secret } }))
}

/// List the deliveries of an endpoint, to see what was sent and what failed.
///
/// Requires `webhooks.manage`. Newest first unless `order=asc`. Kept for 30 days.
#[utoipa::path(
    get,
    path = "/{id}/deliveries",
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
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Result<Page<DeliveryResponse>, Error> {
    principal.require(&WEBHOOKS_MANAGE)?;
    let page = service.deliveries(id, &PageRequest::try_from(query)?).await?;
    Ok(page.map(DeliveryResponse::from))
}
