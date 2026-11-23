//! The write batch of an offline app (step 69 P2, D-20261119-step69-sync): `POST /api/v1/kernel/sync/batch`
//! takes the write commands queued on the device and runs EACH command like a real REST request - through
//! the module's router (authentication, plan-based permissions, agent approval), the idempotency layer,
//! error normalization and translation - so each command goes through exactly the same handler and service
//! as a REST call (architecture rule 8): there is no second write path. One failed command does not block
//! the others.
//!
//! The sub-router contains only the routes of modules that have `sync_resources` (not this route), so there
//! is no reference cycle; it is filled in after the HTTP layer finishes building the shared layers.
//!
//! Same file: `GET /api/v1/kernel/sync/resources` - the record types that can be synced, so the app knows
//! the path of the change stream and the path of the batch write.

use std::sync::{Arc, OnceLock};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, header};
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use crate::access::Principal;
use crate::error::{Error, FieldError};
use crate::http::ValidJson;
use crate::response::{Data, Problem};
use crate::sync::SyncResource;

/// The maximum number of commands in one batch.
const MAX_OPERATIONS: usize = 100;
/// A response body of a command longer than this is not copied into the result.
const MAX_RESULT_BYTES: usize = 4 * 1024 * 1024;

/// The batch request's headers are NOT copied to the sub-commands: body, conditions and key belong to each
/// command.
const OWN_HEADERS: [header::HeaderName; 8] = [
    header::CONTENT_LENGTH,
    header::CONTENT_TYPE,
    header::TRANSFER_ENCODING,
    header::EXPECT,
    header::IF_MATCH,
    header::IF_NONE_MATCH,
    header::IF_MODIFIED_SINCE,
    header::IF_UNMODIFIED_SINCE,
];

/// The state of the two endpoints.
pub(super) struct SyncState {
    /// `(module name, declaration)`.
    pub(super) resources: Vec<(&'static str, SyncResource)>,
    /// The sub-router that runs the batch's commands; filled in at the end of `build`.
    pub(super) router: OnceLock<Router>,
}

pub(super) fn routes(state: Arc<SyncState>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(sync_resources))
        .routes(routes!(sync_batch))
        .with_state(state)
}

/// A kind of record an offline app can keep in sync.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct SyncResourceView {
    /// `<module>.<name>`.
    #[schema(example = "notes.notes")]
    name: String,
    /// The list endpoint of these records; writes in a batch go under this path.
    #[schema(example = "/api/v1/notes")]
    path: String,
    /// The change feed: `GET` it with the `cursor` of the last change you applied.
    #[schema(example = "/api/v1/notes/changes")]
    changes: String,
    /// The permission needed to read them; its scope decides which records you get.
    #[schema(example = "notes.read")]
    read_permission: String,
    description: String,
}

/// List what an offline app can keep in sync.
///
/// Each kind of record has a change feed (`changes`): load the list endpoint once, then ask the
/// feed for what changed, keeping the `cursor` of the last change. Queue your writes on the device
/// and send them with `POST /api/v1/kernel/sync/batch`. Needs only to be signed in.
#[utoipa::path(
    get,
    path = "/sync/resources",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_sync_resources",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The kinds of records that can be kept in sync.", body = Data<Vec<SyncResourceView>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn sync_resources(
    State(state): State<Arc<SyncState>>,
    _principal: Principal,
) -> Data<Vec<SyncResourceView>> {
    let views = state
        .resources
        .iter()
        .map(|(module, resource)| {
            let path = collection(module, resource);
            SyncResourceView {
                name: format!("{module}.{}", resource.name),
                changes: format!("{path}/changes"),
                path,
                read_permission: resource.read.name().to_owned(),
                description: resource.description.to_owned(),
            }
        })
        .collect();
    Data { data: views }
}

/// Writes queued on a device while it was offline.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchRequest {
    /// 1 to 100 writes, run one after the other in this order.
    operations: Vec<BatchOperation>,
}

/// One write, as you would send it to the REST endpoint.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchOperation {
    /// Your id for this write, 1 to 100 characters; its result carries it back.
    #[schema(example = "op-1")]
    id: String,
    method: BatchMethod,
    /// Path of the REST endpoint, under the `path` of a sync resource
    /// (`GET /api/v1/kernel/sync/resources`).
    #[schema(example = "/api/v1/notes/0190f5c2-7b1a-7cde-8f00-123456789abc")]
    path: String,
    /// Sent as the `Idempotency-Key` header. Keep it with the queued write and send the same key
    /// when you send the batch again: a write already done answers what it answered the first time.
    #[schema(example = "8b2f6a3e-5c1d-4e7f-9a0b-1c2d3e4f5a6b")]
    idempotency_key: String,
    /// Sent as the `If-Match` header: the `ETag` (`version`) of the record you changed. If someone
    /// changed it since, the result is `412` with the current record in the body's `current`.
    if_match: Option<String>,
    /// The JSON body of the write, if it has one.
    body: Option<serde_json::Value>,
}

/// HTTP method of a write.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum BatchMethod {
    Post,
    Put,
    Patch,
    Delete,
}

impl BatchMethod {
    fn method(self) -> Method {
        match self {
            BatchMethod::Post => Method::POST,
            BatchMethod::Put => Method::PUT,
            BatchMethod::Patch => Method::PATCH,
            BatchMethod::Delete => Method::DELETE,
        }
    }
}

/// What one write answered.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct BatchResult {
    /// The `id` you gave the write.
    id: String,
    /// The HTTP status the REST endpoint answered, e.g. `201`, `200`, `204`, `412`.
    #[schema(example = 200)]
    status: u16,
    /// The `ETag` header of the answer (version of the record after the write), if any.
    etag: Option<String>,
    /// The `Location` header of the answer (the record created), if any.
    location: Option<String>,
    /// The JSON body of the answer: `{"data": …}`, or a problem (`code`, `detail`…) when the write
    /// failed; `null` when the answer has no body.
    body: Option<serde_json::Value>,
}

/// Send writes queued while offline.
///
/// Runs each write, in order, exactly as its REST endpoint would (same permissions, checks and
/// answers), and answers one result per write; a failed write does not stop the others. Give each
/// write an `idempotency_key` and keep it: sending the batch again does not repeat what was done.
/// Send the record's `ETag` in `if_match`: a write over someone else's change answers `412` with
/// the current record. Each write needs the permission of its endpoint.
#[utoipa::path(
    post,
    path = "/sync/batch",
    extensions(("x-permissions" = json!([]))),
    operation_id = "sync_batch",
    security(("bearer" = [])),
    request_body = BatchRequest,
    responses(
        (status = 200, description = "One result per write, in the order sent.", body = Data<Vec<BatchResult>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "The batch is invalid (no writes, more than 100, a path outside the sync resources…); see `errors`. Nothing was run.", body = Problem),
    )
)]
async fn sync_batch(
    State(state): State<Arc<SyncState>>,
    _principal: Principal,
    parts: Parts,
    ValidJson(input): ValidJson<BatchRequest>,
) -> Result<Data<Vec<BatchResult>>, Error> {
    check(&state.resources, &input)?;
    let router = state.router.get().ok_or_else(|| Error::internal("sync router not ready"))?;
    let mut results = Vec::with_capacity(input.operations.len());
    for operation in input.operations {
        results.push(run(router, &parts, operation).await?);
    }
    Ok(Data { data: results })
}

/// Validates the whole batch before running any command; reports every error at once.
fn check(resources: &[(&'static str, SyncResource)], input: &BatchRequest) -> Result<(), Error> {
    let mut problems = Vec::new();
    if input.operations.is_empty() || input.operations.len() > MAX_OPERATIONS {
        problems.push(
            FieldError::new("operations", "length", "must hold 1 to {max} writes")
                .with_param("max", MAX_OPERATIONS),
        );
    }
    for (index, operation) in input.operations.iter().enumerate() {
        if !(1..=100).contains(&operation.id.chars().count()) {
            problems.push(FieldError::new(
                format!("operations[{index}].id"),
                "length",
                "must be 1 to 100 characters",
            ));
        }
        if !allowed(resources, &operation.path) {
            problems.push(FieldError::new(
                format!("operations[{index}].path"),
                "invalid",
                "must be a path under a sync resource (`GET /api/v1/kernel/sync/resources`)",
            ));
        }
        if operation.idempotency_key.trim().is_empty() {
            problems.push(FieldError::new(
                format!("operations[{index}].idempotency_key"),
                "required",
                "is required: keep one key per queued write",
            ));
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

/// The list path of resource type `resource` of module `module`.
fn collection(module: &str, resource: &SyncResource) -> String {
    format!("{}/{module}{}", super::API_PREFIX, resource.path)
}

/// `path` lies under the path of a syncable type, with no odd segments (`..`, `//`).
fn allowed(resources: &[(&'static str, SyncResource)], path: &str) -> bool {
    let bare = path.split(['?', '#']).next().unwrap_or_default();
    if bare.contains("//") || bare.split('/').any(|segment| segment == "." || segment == "..") {
        return false;
    }
    resources.iter().any(|(module, resource)| {
        let base = collection(module, resource);
        bare == base || bare.strip_prefix(&base).is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Runs one command through the sub-router with the identity of the batch request.
async fn run(
    router: &Router,
    parts: &Parts,
    operation: BatchOperation,
) -> Result<BatchResult, Error> {
    let body = match &operation.body {
        Some(value) => Body::from(serde_json::to_vec(value).map_err(Error::internal)?),
        None => Body::empty(),
    };
    let mut request = Request::new(body);
    *request.method_mut() = operation.method.method();
    *request.uri_mut() = operation.path.parse().map_err(Error::internal)?;
    *request.version_mut() = parts.version;
    // Danh tính, khách, ngôn ngữ đi theo request lô; thân và điều kiện là của từng lệnh.
    let headers = request.headers_mut();
    for (name, value) in &parts.headers {
        if !OWN_HEADERS.contains(name) && name.as_str() != "idempotency-key" {
            headers.append(name.clone(), value.clone());
        }
    }
    if operation.body.is_some() {
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    let key = HeaderValue::from_str(&operation.idempotency_key).map_err(|_| {
        Error::validation(vec![FieldError::new(
            "idempotency_key",
            "invalid",
            "must be visible ASCII characters",
        )])
    });
    match key {
        Ok(key) => {
            headers.insert("idempotency-key", key);
        }
        Err(error) => return Ok(failed(operation.id, &error)),
    }
    if let Some(version) = &operation.if_match {
        match HeaderValue::from_str(version) {
            Ok(value) => {
                headers.insert(header::IF_MATCH, value);
            }
            Err(_) => return Ok(failed(operation.id, &crate::version::changed())),
        }
    }
    // Phần mở rộng (bộ xác thực, sổ database, địa chỉ client…) do các lớp ngoài gắn vào request lô.
    *request.extensions_mut() = parts.extensions.clone();
    let response = router.clone().oneshot(request).await.map_err(Error::internal)?;
    let status = response.status().as_u16();
    // Đọc header trước khi lấy thân: closure mượn response không được sống qua `await`
    // (thân response không `Sync`).
    let (etag, location) = {
        let text = |name: header::HeaderName| {
            response.headers().get(name).and_then(|value| value.to_str().ok()).map(str::to_owned)
        };
        (text(header::ETAG), text(header::LOCATION))
    };
    let bytes = axum::body::to_bytes(response.into_body(), MAX_RESULT_BYTES)
        .await
        .map_err(Error::internal)?;
    let body = if bytes.is_empty() { None } else { serde_json::from_slice(&bytes).ok() };
    Ok(BatchResult { id: operation.id, status, etag, location, body })
}

/// The result of a command that failed before reaching the endpoint.
fn failed(id: String, error: &Error) -> BatchResult {
    let problem = Problem::from(error);
    BatchResult {
        id,
        status: problem.status,
        etag: None,
        location: None,
        body: serde_json::to_value(problem).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::Permission;

    #[test]
    fn chi_nhan_duong_dan_duoi_loai_dong_bo_duoc() {
        const READ: Permission = Permission::new("notes.read", "Read.");
        let resources = [("notes", SyncResource::new("notes", "", READ, "Notes."))];
        assert!(allowed(&resources, "/api/v1/notes"));
        assert!(allowed(&resources, "/api/v1/notes/abc?x=1"));
        assert!(!allowed(&resources, "/api/v1/notesx"));
        assert!(!allowed(&resources, "/api/v1/notes/../kernel/sync/batch"));
        assert!(!allowed(&resources, "/api/v1/notes//x"));
        assert!(!allowed(&resources, "/api/v1/kernel/sync/batch"));
    }
}
