//! Endpoint SCIM `Groups` (RFC 7644) dưới `/api/v1/auth/scim/v2/Groups`: cùng cách làm với
//! `Users` ở `scim.rs` (token SCIM, `application/scim+json`, lỗi SCIM).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::HttpState;
use super::scim::{
    ScimErrorBody, ScimListQuery, ScimListResponse, ScimPatch, caller, created, json_body, respond,
};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new().routes(routes!(scim_list_groups, scim_create_group)).routes(routes!(
        scim_get_group,
        scim_replace_group,
        scim_patch_group,
        scim_delete_group
    ))
}

/// A SCIM Group (RFC 7643 §4.2): a group of the identity provider. Mapping rules of the tenant
/// (`/api/v1/auth/scim/group-mappings`) turn its members into holders of roles at units.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = json!({
    "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
    "displayName": "Sales",
    "externalId": "9b1f…",
    "members": [{"value": "0190f2c4-6a1e-7c3b-9d2e-1f4a5b6c7d8e"}]
}))]
// Chỉ để tài liệu OpenAPI: thân thật dựng bằng `serde_json` ở `crate::scim`.
#[allow(dead_code)]
pub(crate) struct ScimGroup {
    schemas: Option<Vec<String>>,
    /// Id given by Autoxide; read-only.
    id: Option<String>,
    /// Name of the group, unique in the tenant (compared without case). Required.
    #[serde(rename = "displayName")]
    display_name: String,
    /// Id of the group at the identity provider; unique in the tenant.
    #[serde(rename = "externalId")]
    external_id: Option<String>,
    /// Members: `value` is the id of a SCIM User of the tenant.
    members: Option<Vec<Value>>,
}

/// List or find SCIM Groups.
///
/// Filter by `displayName`, `externalId`, `id` or `members` (`eq` only). `excludedAttributes=members`
/// leaves the members out, as Entra ID asks.
#[utoipa::path(
    get,
    path = "/scim/v2/Groups",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_list_groups",
    security(("bearer" = [])),
    params(ScimListQuery),
    responses(
        (status = 200, description = "One page of Groups.", body = ScimListResponse, content_type = "application/scim+json"),
        (status = 400, description = "The filter cannot be read or is not supported; `scimType` `invalidFilter`.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`).", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_list_groups(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Query(query): Query<ScimListQuery>,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        let (start, count) = query.page();
        let members = !query.excludes("members");
        state.scim.list_groups(&caller, query.filter.as_deref(), start, count, members).await
    };
    respond(StatusCode::OK, result.await)
}

/// Create a SCIM Group.
///
/// Members must be SCIM Users of the tenant. Members get the roles the tenant's mapping rules give
/// for this group.
#[utoipa::path(
    post,
    path = "/scim/v2/Groups",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_create_group",
    security(("bearer" = [])),
    request_body(content = ScimGroup, content_type = "application/scim+json"),
    responses(
        (status = 201, description = "The Group was created; `Location` is its address.", body = ScimGroup, content_type = "application/scim+json"),
        (status = 400, description = "The body is not JSON (`invalidSyntax`) or a value is wrong, e.g. no `displayName` or an unknown member (`invalidValue`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`), or giving the roles would exceed the plan's staff limit.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 409, description = "`displayName` or `externalId` is used by another Group (`uniqueness`), or a change would leave the tenant without an active administrator.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_create_group(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.create_group(&caller, &json_body(&body)?).await
    };
    created(result.await)
}

/// Get a SCIM Group.
#[utoipa::path(
    get,
    path = "/scim/v2/Groups/{id}",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_get_group",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id of the Group."), ScimListQuery),
    responses(
        (status = 200, description = "The Group.", body = ScimGroup, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 404, description = "No Group of this tenant has this id.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_get_group(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<ScimListQuery>,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.get_group(&caller, &id, !query.excludes("members")).await
    };
    respond(StatusCode::OK, result.await)
}

/// Replace a SCIM Group.
///
/// The name, `externalId` and the whole member list are replaced; members who leave lose the
/// roles the group gave them.
#[utoipa::path(
    put,
    path = "/scim/v2/Groups/{id}",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_replace_group",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id of the Group.")),
    request_body(content = ScimGroup, content_type = "application/scim+json"),
    responses(
        (status = 200, description = "The Group after the change.", body = ScimGroup, content_type = "application/scim+json"),
        (status = 400, description = "The body is not JSON (`invalidSyntax`) or a value is wrong (`invalidValue`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`), or giving the roles would exceed the plan's staff limit.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 404, description = "No Group of this tenant has this id.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 409, description = "A value is used by another Group (`uniqueness`), or a change would leave the tenant without an active administrator.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_replace_group(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.replace_group(&caller, &id, &json_body(&body)?).await
    };
    respond(StatusCode::OK, result.await)
}

/// Change a SCIM Group with PATCH operations.
///
/// RFC 7644 §3.5.2, e.g. `add` / `remove` `members`, `remove` `members[value eq "…"]`, `replace`
/// `displayName`. Membership changes reach the members' roles at once.
#[utoipa::path(
    patch,
    path = "/scim/v2/Groups/{id}",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_patch_group",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id of the Group.")),
    request_body(content = ScimPatch, content_type = "application/scim+json"),
    responses(
        (status = 200, description = "The Group after the change.", body = ScimGroup, content_type = "application/scim+json"),
        (status = 400, description = "The operations cannot be applied: `invalidSyntax`, `invalidPath`, `noTarget`, `mutability` or `invalidValue`.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`), or giving the roles would exceed the plan's staff limit.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 404, description = "No Group of this tenant has this id.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 409, description = "A value is used by another Group (`uniqueness`), or a change would leave the tenant without an active administrator.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_patch_group(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.patch_group(&caller, &id, &json_body(&body)?).await
    };
    respond(StatusCode::OK, result.await)
}

/// Delete a SCIM Group.
///
/// Its mapping rules go with it; its members lose the roles it gave them.
#[utoipa::path(
    delete,
    path = "/scim/v2/Groups/{id}",
    // Không đăng nhập Autoxide: xác thực bằng token SCIM của khách (D-20261121-scim).
    extensions(("x-public" = json!(true))),
    operation_id = "scim_delete_group",
    security(("bearer" = [])),
    params(("id" = String, Path, description = "Id of the Group.")),
    responses(
        (status = 204, description = "The Group was deleted."),
        (status = 401, description = "No SCIM token, or it is unknown or revoked.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 403, description = "The tenant's plan does not include SCIM (`auth.scim`).", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 404, description = "No Group of this tenant has this id.", body = ScimErrorBody, content_type = "application/scim+json"),
        (status = 409, description = "Removing the roles would leave the tenant without an active administrator.", body = ScimErrorBody, content_type = "application/scim+json"),
    )
)]
async fn scim_delete_group(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let result = async {
        let caller = caller(&state, &headers).await?;
        state.scim.delete_group(&caller, &id).await
    };
    match result.await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error.into_response(),
    }
}
