//! Endpoint SCIM 2.0 (RFC 7644) dưới `/api/v1/auth/scim/v2`, cho hệ thống danh tính của khách
//! (Entra ID, Okta, Google Workspace). Khác các endpoint khác của Autoxide có chủ đích
//! (D-20261121-scim): thân `application/scim+json`, danh sách `ListResponse`, phân trang
//! `startIndex` / `count`, lỗi theo lược đồ lỗi SCIM, xác thực bằng token SCIM của khách (không
//! phải access token / API key). Handler chỉ đọc request, gọi `ScimService`, trả JSON.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::HttpState;
use crate::scim::discovery::{self, MAX_RESULTS};
use crate::scim::error::{SCIM_CONTENT_TYPE, ScimError, ScimType};
use crate::scim::{BASE_PATH, ScimCaller};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(scim_list_users, scim_create_user))
        .routes(routes!(scim_get_user, scim_replace_user, scim_patch_user, scim_delete_user))
        .routes(routes!(scim_service_provider_config))
        .routes(routes!(scim_resource_types))
        .routes(routes!(scim_schemas))
}

/// A SCIM User (RFC 7643 §4.1), as Autoxide stores it: the staff account of the tenant.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = json!({
    "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
    "userName": "ada@example.com",
    "externalId": "00u1abcd",
    "name": {"givenName": "Ada", "familyName": "Lovelace"},
    "emails": [{"value": "ada@example.com", "type": "work", "primary": true}],
    "active": true
}))]
// Chỉ để tài liệu OpenAPI: thân thật dựng bằng `serde_json` ở `crate::scim`.
#[allow(dead_code)]
pub(crate) struct ScimUser {
    /// `urn:ietf:params:scim:schemas:core:2.0:User`.
    schemas: Option<Vec<String>>,
    /// Id given by Autoxide (the account id); read-only.
    id: Option<String>,
    /// Unique sign-in name at the identity provider, compared without case. Required.
    #[serde(rename = "userName")]
    user_name: String,
    /// Id of the person at the identity provider; unique in the tenant, compared with case.
    #[serde(rename = "externalId")]
    external_id: Option<String>,
    /// Name shown in Autoxide; else `name.formatted`, else given and family name, else `userName`.
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    /// `givenName`, `familyName`, `formatted`.
    name: Option<Value>,
    /// Work email: the `primary` one, else the first; when absent, `userName` must be an email.
    emails: Option<Vec<Value>>,
    /// `false` disables the account at once: its sessions, refresh tokens and API keys are
    /// revoked in the same transaction. `"True"` / `"False"` strings are accepted.
    active: Option<bool>,
    /// Groups the user is in (read-only; change them through `/Groups`).
    groups: Option<Vec<Value>>,
}

/// A SCIM list (`urn:ietf:params:scim:api:messages:2.0:ListResponse`, RFC 7644 §3.4.2).
#[derive(Debug, Deserialize, ToSchema)]
// Chỉ để tài liệu OpenAPI: thân thật dựng bằng `serde_json` ở `crate::scim`.
#[allow(dead_code)]
pub(crate) struct ScimListResponse {
    schemas: Vec<String>,
    /// How many resources match the filter, on every page.
    #[serde(rename = "totalResults")]
    total_results: u64,
    /// 1-based index of the first resource of this page.
    #[serde(rename = "startIndex")]
    start_index: u64,
    /// How many resources this page holds.
    #[serde(rename = "itemsPerPage")]
    items_per_page: u64,
    /// The resources of this page.
    #[serde(rename = "Resources")]
    resources: Vec<Value>,
}

/// A SCIM error (`urn:ietf:params:scim:api:messages:2.0:Error`, RFC 7644 §3.12).
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = json!({
    "schemas": ["urn:ietf:params:scim:api:messages:2.0:Error"],
    "status": "409",
    "scimType": "uniqueness",
    "detail": "userName `ada@example.com` is already used by another User."
}))]
// Chỉ để tài liệu OpenAPI: thân thật dựng bằng `serde_json` ở `crate::scim`.
#[allow(dead_code)]
pub(crate) struct ScimErrorBody {
    schemas: Vec<String>,
    /// The HTTP status, as a string.
    status: String,
    /// `invalidFilter`, `uniqueness`, `mutability`, `invalidSyntax`, `invalidPath`, `noTarget`
    /// or `invalidValue`, when one applies.
    #[serde(rename = "scimType")]
    scim_type: Option<String>,
    /// What went wrong, for people.
    detail: String,
}

/// A SCIM PATCH request (`urn:ietf:params:scim:api:messages:2.0:PatchOp`, RFC 7644 §3.5.2).
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = json!({
    "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
    "Operations": [{"op": "replace", "path": "active", "value": false}]
}))]
// Chỉ để tài liệu OpenAPI: thân thật dựng bằng `serde_json` ở `crate::scim`.
#[allow(dead_code)]
pub(crate) struct ScimPatch {
    schemas: Vec<String>,
    /// `op` (`add`, `replace`, `remove`, any case), `path` (`attr`, `attr.sub`,
    /// `attr[filter]`, `attr[filter].sub`) and `value`. Without `path`, `value` is an object of
    /// attributes.
    #[serde(rename = "Operations")]
    operations: Vec<Value>,
}

/// Paging and filtering of a SCIM list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ScimListQuery {
    /// SCIM filter: `eq`, `ne`, `co`, `sw`, `ew`, `pr`, joined with `and` / `or` and
    /// parentheses. Users: `userName`, `externalId`, `emails`, `active`, `id`; Groups:
    /// `displayName`, `externalId`, `id`, `members` (`eq`).
    #[param(example = "userName eq \"ada@example.com\"")]
    pub(super) filter: Option<String>,
    /// 1-based index of the first result; default 1.
    #[serde(rename = "startIndex")]
    #[param(rename = "startIndex")]
    start_index: Option<i64>,
    /// Results per page; default and most 200. `0` returns only `totalResults`.
    count: Option<i64>,
    /// Ignored: every attribute is returned.
    #[allow(dead_code)]
    attributes: Option<String>,
    /// Ignored, except `groups` on Users and `members` on Groups.
    #[serde(rename = "excludedAttributes")]
    #[param(rename = "excludedAttributes")]
    excluded_attributes: Option<String>,
}

impl ScimListQuery {
    /// `startIndex` < 1 là 1; `count` < 0 là 0, quá trần thì bằng trần (RFC 7644 §3.4.2.4).
    pub(super) fn page(&self) -> (u64, u64) {
        let start = u64::try_from(self.start_index.unwrap_or(1).max(1)).unwrap_or(1);
        let count = u64::try_from(self.count.unwrap_or(MAX_RESULTS as i64).max(0)).unwrap_or(0);
        (start, count.min(MAX_RESULTS))
    }

    /// `excludedAttributes` có `name` (không phân biệt hoa thường).
    pub(super) fn excludes(&self, name: &str) -> bool {
        self.excluded_attributes
            .as_deref()
            .is_some_and(|list| list.split(',').any(|item| item.trim().eq_ignore_ascii_case(name)))
    }
}

/// Response SCIM: `status` và thân JSON `application/scim+json`, hay lỗi SCIM.
pub(super) fn respond(status: StatusCode, result: Result<Value, ScimError>) -> Response {
    match result {
        Ok(body) => {
            let mut response = (status, body.to_string()).into_response();
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(SCIM_CONTENT_TYPE));
            response
        }
        Err(error) => error.into_response(),
    }
}

/// Như [`respond`] với `201 Created` và header `Location` (`meta.location`).
pub(super) fn created(result: Result<Value, ScimError>) -> Response {
    let location = result
        .as_ref()
        .ok()
        .and_then(|body| body["meta"]["location"].as_str())
        .and_then(|location| HeaderValue::from_str(location).ok());
    let mut response = respond(StatusCode::CREATED, result);
    if let Some(location) = location {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
}

/// Xác thực token SCIM của request.
pub(super) async fn caller(
    state: &HttpState,
    headers: &HeaderMap,
) -> Result<ScimCaller, ScimError> {
    let authorization = headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok());
    state.scim.authenticate(authorization).await
}

/// Thân JSON của request (`application/scim+json` hay `application/json`).
pub(super) fn json_body(body: &Bytes) -> Result<Value, ScimError> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| ScimError::bad(ScimType::InvalidSyntax, "The body must be a JSON object."))
}

/// List or find SCIM Users.
///
/// For the identity provider of the tenant, with a SCIM token (`Authorization: Bearer axscim1_…`).
/// Entra ID and Okta look a person up with `filter=userName eq "…"` before creating them.
#[utoipa::path(
    get,
    path = "/scim/v2/Users",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_list_users",
    security(("bearer" = [])),
    params(ScimListQuery),
    responses(
        (status = 200, description = "One page of Users.", body = ScimListResponse, content_type = "application/scim+json"),
        (status = 400, description = "The filter cannot be read or uses an attribute or operator that is not supported; `scimType` `invalidFilter`.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`).", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_list_users(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Query(query): Query<ScimListQuery>,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        let (start, count) = query.page();
        let mut list =
            state.scim.list_users(&caller, query.filter.as_deref(), start, count).await?;
        // `excludedAttributes=groups`: bỏ nhóm khỏi mỗi User (các thuộc tính khác luôn trả).
        if query.excludes("groups")
            && let Some(users) = list["Resources"].as_array_mut()
        {
            for user in users.iter_mut().filter_map(Value::as_object_mut) {
                user.remove("groups");
            }
        }
        Ok(list)
    };
    respond(StatusCode::OK, result.await)
}

/// Create a SCIM User (provision a staff account).
///
/// Creates the staff account without a password (the person signs in through the tenant's sign-in
/// provider, or sets a password by reset). An existing staff account with the same email and no
/// SCIM link is taken over instead of duplicated. Unknown attributes are ignored.
#[utoipa::path(
    post,
    path = "/scim/v2/Users",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_create_user",
    security(("bearer" = [])),
    request_body(content = ScimUser, content_type = "application/scim+json"),
    responses(
        (status = 201, description = "The User was created; `Location` is its address.", body = ScimUser, content_type = "application/scim+json"),
        (status = 400, description = "The body is not JSON (`invalidSyntax`) or a value is wrong, e.g. no `userName` or no valid email (`invalidValue`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`), or the account has permissions that the SCIM token's creator does not have now (the body names no `scimType`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 409, description = "`userName`, `externalId` or the email is used by another User; `scimType` `uniqueness`.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_create_user(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.create_user(&caller, &json_body(&body)?).await
    };
    created(result.await)
}

/// Get a SCIM User.
#[utoipa::path(
    get,
    path = "/scim/v2/Users/{id}",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_get_user",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id of the User (the account id).")),
    responses(
        (status = 200, description = "The User.", body = ScimUser, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 404, description = "No User of this tenant has this id.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_get_user(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.get_user(&caller, &id).await
    };
    respond(StatusCode::OK, result.await)
}

/// Replace a SCIM User.
///
/// Every attribute is replaced; one left out is cleared (e.g. `externalId`). `active: false`
/// disables the account and revokes its sessions and API keys at once.
#[utoipa::path(
    put,
    path = "/scim/v2/Users/{id}",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_replace_user",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id of the User (the account id).")),
    request_body(content = ScimUser, content_type = "application/scim+json"),
    responses(
        (status = 200, description = "The User after the change.", body = ScimUser, content_type = "application/scim+json"),
        (status = 400, description = "The body is not JSON (`invalidSyntax`) or a value is wrong (`invalidValue`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`), or the account has permissions that the SCIM token's creator does not have now (the body names no `scimType`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 404, description = "No User of this tenant has this id.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 409, description = "A value is used by another User (`uniqueness`), or disabling would leave the tenant without an active administrator.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_replace_user(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.replace_user(&caller, &id, &json_body(&body)?).await
    };
    respond(StatusCode::OK, result.await)
}

/// Change a SCIM User with PATCH operations.
///
/// RFC 7644 §3.5.2: `add`, `replace`, `remove`, with or without `path`. `replace` of `active`
/// with `false` (how Entra ID and Okta deprovision) disables the account and revokes its sessions
/// and API keys at once.
#[utoipa::path(
    patch,
    path = "/scim/v2/Users/{id}",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_patch_user",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id of the User (the account id).")),
    request_body(content = ScimPatch, content_type = "application/scim+json"),
    responses(
        (status = 200, description = "The User after the change.", body = ScimUser, content_type = "application/scim+json"),
        (status = 400, description = "The operations cannot be applied: `invalidSyntax`, `invalidPath`, `noTarget`, `mutability` (e.g. `id`) or `invalidValue`.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`), or the account has permissions that the SCIM token's creator does not have now (the body names no `scimType`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 404, description = "No User of this tenant has this id.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 409, description = "A value is used by another User (`uniqueness`), or disabling would leave the tenant without an active administrator.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_patch_user(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.patch_user(&caller, &id, &json_body(&body)?).await
    };
    respond(StatusCode::OK, result.await)
}

/// Delete a SCIM User.
///
/// Deletes the account the same way as any account deletion: other modules are told to delete
/// what they keep about the person. To only block access, PATCH `active` to `false` instead. The
/// token's creator must hold every permission the account has.
#[utoipa::path(
    delete,
    path = "/scim/v2/Users/{id}",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_delete_user",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id of the User (the account id).")),
    responses(
        (status = 204, description = "The User and its account were deleted."),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`), or the account has permissions that the SCIM token's creator does not have now (the body names no `scimType`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 404, description = "No User of this tenant has this id.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 409, description = "The account is the last active administrator of the tenant.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_delete_user(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.delete_user(&caller, &id).await
    };
    match result.await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error.into_response(),
    }
}

/// Describe what this SCIM server supports.
///
/// RFC 7644 §4: PATCH and filters yes; bulk, sort, ETags and password change no. No token needed.
#[utoipa::path(
    get,
    path = "/scim/v2/ServiceProviderConfig",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_service_provider_config",
    responses(
        (status = 200, description = "The service provider configuration.", body = Object, content_type = "application/scim+json"),
    )
)]
async fn scim_service_provider_config() -> Response {
    respond(StatusCode::OK, Ok(discovery::service_provider_config(BASE_PATH)))
}

/// List the SCIM resource types (`User`, `Group`).
///
/// No token needed.
#[utoipa::path(
    get,
    path = "/scim/v2/ResourceTypes",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_resource_types",
    responses(
        (status = 200, description = "The resource types.", body = ScimListResponse, content_type = "application/scim+json"),
    )
)]
async fn scim_resource_types() -> Response {
    respond(StatusCode::OK, Ok(discovery::list(discovery::resource_types(BASE_PATH))))
}

/// List the SCIM schemas, with the attributes Autoxide keeps.
///
/// No token needed.
#[utoipa::path(
    get,
    path = "/scim/v2/Schemas",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_schemas",
    responses(
        (status = 200, description = "The schemas of `User` and `Group`.", body = ScimListResponse, content_type = "application/scim+json"),
    )
)]
async fn scim_schemas() -> Response {
    respond(StatusCode::OK, Ok(discovery::list(discovery::schemas(BASE_PATH))))
}
