//! Endpoint API key: người dùng tạo, xem và thu hồi key của chính mình.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::api_key::{ApiKey, CreatedApiKey, NewApiKey};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_api_keys, create_api_key))
        .routes(routes!(revoke_api_key))
}

/// An API key, without its secret.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ApiKeyResponse {
    id: Uuid,
    /// What the key is for.
    #[schema(example = "nightly report agent")]
    name: String,
    /// The first characters of the key, to recognize it.
    #[schema(example = "axk_3q9ZbXcV")]
    prefix: String,
    /// Permissions the key may use, sorted by name. The key never has more than its
    /// owner has at the moment of the request.
    #[schema(example = json!(["auth.users.read"]))]
    permissions: Vec<String>,
    /// When the key stops working; `null`: never.
    expires_at: Option<DateTime<Utc>>,
    /// Last time the key was used, to the minute; `null`: never used.
    last_used_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<ApiKey> for ApiKeyResponse {
    fn from(key: ApiKey) -> Self {
        ApiKeyResponse {
            id: key.id,
            name: key.name,
            prefix: key.prefix,
            permissions: key.permissions,
            expires_at: key.expires_at,
            last_used_at: key.last_used_at,
            created_at: key.created_at,
        }
    }
}

/// A new API key, with its secret. The secret is shown only here.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct CreatedApiKeyResponse {
    /// The key. Send it as `Authorization: Bearer <key>`. Store it now: it cannot be
    /// shown again.
    #[schema(example = "axk_3q9ZbXcV…")]
    key: String,
    #[serde(flatten)]
    details: ApiKeyResponse,
}

/// A key to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct CreateApiKeyRequest {
    /// What the key is for, 1 to 100 characters.
    #[schema(example = "nightly report agent")]
    name: String,
    /// Permissions the key may use: at least one, each one you have yourself (see
    /// `GET /api/v1/auth/me`). Give a key only what its job needs.
    #[schema(example = json!(["auth.users.read"]))]
    permissions: Vec<String>,
    /// When the key stops working (RFC 3339, in the future). Omit for a key that does
    /// not expire.
    expires_at: Option<DateTime<Utc>>,
    /// Your current password: a stolen access token alone must not create a key that
    /// outlives it.
    current_password: String,
}

impl From<CreateApiKeyRequest> for NewApiKey {
    fn from(input: CreateApiKeyRequest) -> Self {
        NewApiKey {
            name: input.name,
            permissions: input.permissions,
            expires_at: input.expires_at,
            current_password: input.current_password,
        }
    }
}

impl From<CreatedApiKey> for CreatedApiKeyResponse {
    fn from(created: CreatedApiKey) -> Self {
        CreatedApiKeyResponse { key: created.secret, details: created.key.into() }
    }
}

/// List your API keys.
///
/// Any signed-in user. Newest first unless `order=asc`. Secrets are never listed.
#[utoipa::path(
    get,
    path = "/api-keys",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_api_keys",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your keys.", body = Page<ApiKeyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_api_keys(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<ApiKeyResponse>, Error> {
    let page = state.api_keys.list(principal.user_id(), &PageRequest::try_from(query)?).await?;
    Ok(page.map(ApiKeyResponse::from))
}

/// Create an API key for a program or an AI agent.
///
/// Signed in with a password session only: an API key cannot create keys. The key
/// acts as you, limited to the permissions listed. The response carries the key
/// itself, shown only once.
#[utoipa::path(
    post,
    path = "/api-keys",
    extensions(("x-permissions" = json!([]))),
    operation_id = "create_api_key",
    security(("bearer" = [])),
    request_body = CreateApiKeyRequest,
    responses(
        (status = 201, description = "The key was created; `key` holds the secret.", body = Data<CreatedApiKeyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.api_key_cannot_create_keys`.", body = Problem),
        (status = 409, description = "You have too many keys; code `auth.api_key_limit`.", body = Problem),
        (status = 422, description = "A value is invalid, e.g. a permission you do not have (`not_granted`); see `errors`.", body = Problem),
    )
)]
async fn create_api_key(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    ValidJson(input): ValidJson<CreateApiKeyRequest>,
) -> Result<Created<CreatedApiKeyResponse>, Error> {
    let created = state.api_keys.create(&principal, input.into()).await?;
    Ok(Created::new(format!("/auth/api-keys/{}", created.key.id), created.into()))
}

/// Revoke one of your API keys.
///
/// The key stops working at once.
#[utoipa::path(
    delete,
    path = "/api-keys/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "revoke_api_key",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the key.")),
    responses(
        (status = 204, description = "The key was revoked."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no key with this id; code `auth.api_key_not_found`.", body = Problem),
    )
)]
async fn revoke_api_key(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    state.api_keys.revoke(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::tenancy::TenantContext;
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::api::{Accounts, NewAccount};
    use crate::http::test_support::{PASSWORD, app, bearer, call};

    #[tokio::test]
    async fn tao_dung_va_thu_hoi_api_key() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(services.users.as_ref(), &TenantContext::legacy_default(&services.database), root).await.unwrap();
            let session = bearer(&app, "root@example.com", PASSWORD).await;

            // Thiếu hay sai mật khẩu hiện tại: không tạo được (token lộ không đủ).
            let wrong = json!({"name": "agent", "permissions": ["auth.users.read"], "current_password": "wrong one"});
            let (status, _, problem) = call(&app, "POST", "/api-keys", Some(&session), Some(wrong)).await;
            assert_eq!((status, problem["code"].as_str()), (StatusCode::BAD_REQUEST, Some("auth.wrong_password")));
            let missing = json!({"name": "agent", "permissions": ["auth.users.read"]});
            let (status, _, _) = call(&app, "POST", "/api-keys", Some(&session), Some(missing)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

            let body = json!({"name": "agent", "permissions": ["auth.users.read"], "current_password": PASSWORD});
            let (status, headers, created) =
                call(&app, "POST", "/api-keys", Some(&session), Some(body.clone())).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            // Secret chỉ trả một lần: không ai được lưu response (D-149).
            assert_eq!(headers["cache-control"], "no-store");
            let key = created["data"]["key"].as_str().unwrap().to_owned();
            assert!(key.starts_with("axk_"));
            assert_eq!(created["data"]["permissions"], json!(["auth.users.read"]));
            let id = created["data"]["id"].as_str().unwrap().to_owned();

            // Máy gọi API bằng key, không cần đăng nhập.
            let with_key = format!("Bearer {key}");
            let (status, _, users) = call(&app, "GET", "/users", Some(&with_key), None).await;
            assert_eq!(status, StatusCode::OK, "{users}");
            let (status, _, denied) = call(&app, "GET", "/roles", Some(&with_key), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(denied["code"], "permission_denied");
            let (status, _, nested) =
                call(&app, "POST", "/api-keys", Some(&with_key), Some(body)).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(nested["code"], "auth.api_key_cannot_create_keys");

            let (status, _, list) = call(&app, "GET", "/api-keys", Some(&session), None).await;
            assert_eq!(status, StatusCode::OK, "{list}");
            assert!(list["data"][0].get("key").is_none());
            assert_eq!(list["data"][0]["prefix"], json!(&key[..12]));

            let path = format!("/api-keys/{id}");
            let (status, _, _) = call(&app, "DELETE", &path, Some(&session), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let (status, _, gone) = call(&app, "GET", "/users", Some(&with_key), None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(gone["code"], "auth.invalid_api_key");
        })
        .await
        .unwrap();
    }
}
