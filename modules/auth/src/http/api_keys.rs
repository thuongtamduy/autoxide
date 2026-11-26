//! Endpoint API key: người dùng tạo, xem và thu hồi key của chính mình.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::api_key::{ApiKey, CreatedApiKey, NewApiKey, Reauth};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_api_keys, create_api_key))
        .routes(routes!(revoke_api_key))
        .routes(routes!(list_all_api_keys))
        .routes(routes!(revoke_any_api_key))
        .routes(routes!(approve_api_key))
}

/// An API key of any user, for administrators.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ManagedApiKeyResponse {
    #[serde(flatten)]
    key: ApiKeyResponse,
    /// The user the key acts as.
    user_id: Uuid,
    /// Display name of that user; `null` when the account no longer exists.
    #[schema(example = "Ada Lovelace")]
    user_name: Option<String>,
    /// Email of that user; `null` like `user_name`.
    #[schema(example = "ada@example.com")]
    user_email: Option<String>,
}

/// Which keys to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AllApiKeysQuery {
    /// Only the keys of this user.
    user_id: Option<Uuid>,
    /// Only keys in this state: `active` or `pending` (waiting for approval). Callers with
    /// `auth.api_keys.approve` but not `auth.api_keys.manage` always get `pending` keys only.
    #[param(example = "pending")]
    status: Option<String>,
}

/// List the API keys of every user.
///
/// Requires `auth.api_keys.manage`, or `auth.api_keys.approve` to list the keys waiting for
/// approval. Newest first unless `order=asc`, with the owner's name and
/// email; secrets are never listed. Revoke one with `DELETE /api/v1/auth/all-api-keys/{id}`.
#[utoipa::path(
    get,
    path = "/all-api-keys",
    extensions(("x-permissions" = json!(["auth.api_keys.manage", "auth.api_keys.approve"]))),
    operation_id = "list_all_api_keys",
    security(("bearer" = [])),
    params(AllApiKeysQuery, PageQuery),
    responses(
        (status = 200, description = "One page of keys.", body = Page<ManagedApiKeyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.api_keys.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid, e.g. `user_id` is not a UUID; code `validation_failed`, every problem in `errors`.", body = Problem),
    )
)]
async fn list_all_api_keys(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<AllApiKeysQuery>,
    Query(query): Query<PageQuery>,
) -> Result<Page<ManagedApiKeyResponse>, Error> {
    let page = PageRequest::try_from(query)?;
    let status = filter.status.as_deref();
    if status.is_some_and(|status| !["active", "pending"].contains(&status)) {
        return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
            "status",
            "invalid",
            "must be `active` or `pending`",
        )]));
    }
    let page = state.api_keys.list_all(&tenant, &principal, filter.user_id, status, &page).await?;
    let mut owners: Vec<Uuid> = page.data.iter().map(|key| key.user_id).collect();
    owners.sort_unstable();
    owners.dedup();
    let users = state.users.by_ids(&tenant, &owners).await?;
    Ok(page.map(|key| {
        let owner = users.get(&key.user_id);
        ManagedApiKeyResponse {
            user_id: key.user_id,
            user_name: owner.map(|user| user.name.clone()),
            user_email: owner.map(|user| user.email.clone()),
            key: key.into(),
        }
    }))
}

/// Revoke an API key of any user.
///
/// Requires `auth.api_keys.manage`, and every permission of the key's owner (a staff member)
/// unless you revoke one of your own keys from a password session. The key stops working at once; the audit log records who revoked it.
#[utoipa::path(
    delete,
    path = "/all-api-keys/{id}",
    extensions(("x-permissions" = json!(["auth.api_keys.manage"]))),
    operation_id = "revoke_any_api_key",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the key.")),
    responses(
        (status = 204, description = "The key was revoked."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.api_keys.manage` (code `permission_denied`), or the key's owner has permissions you do not have (code `auth.target_has_more_permissions`).", body = Problem),
        (status = 404, description = "No key has this id; code `auth.api_key_not_found`.", body = Problem),
    )
)]
async fn revoke_any_api_key(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    state.api_keys.revoke_any(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// An API key, without its secret.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ApiKeyResponse {
    id: Uuid,
    /// What the key is for.
    #[schema(example = "nightly report agent")]
    name: String,
    /// A few characters of the key, to recognize it: `axk1_…` and the first 8 characters of the
    /// key's random part (after the tenant part). Keys created in older versions (`axk_…`) show
    /// their first 12 characters.
    #[schema(example = "axk1_…3q9ZbXcV")]
    prefix: String,
    /// Permissions the key may use, sorted by name. The key never has more than its
    /// owner has at the moment of the request.
    #[schema(example = json!(["auth.users.read"]))]
    permissions: Vec<String>,
    /// The organization unit the key is bound to: it acts only on data of this unit and the
    /// units below it. The root unit (the id of the tenant) means no limit.
    org_id: Uuid,
    /// When the key stops working. Every new key has one; `null` only for data that was never
    /// upgraded.
    expires_at: Option<Timestamp>,
    /// IP addresses and ranges (CIDR) the key may be used from; empty: any address. A request
    /// from elsewhere gets 403 `network_not_allowed`.
    #[schema(example = json!(["203.0.113.0/24"]))]
    allowed_ips: Vec<String>,
    /// The key this one replaced (`POST /api/v1/auth/api-keys/{id}/rotate`); `null` for a key
    /// created from scratch.
    rotated_from: Option<Uuid>,
    /// `active` (works) or `pending` (waits for an administrator's approval: requests with it get
    /// 401 `auth.api_key_pending`).
    #[schema(example = "active")]
    status: String,
    /// Who approved the key and when; `null` when it needed no approval.
    approved_by: Option<Uuid>,
    approved_at: Option<Timestamp>,
    /// Last time the key was used, to the minute; `null`: never used.
    last_used_at: Option<Timestamp>,
    created_at: Timestamp,
    /// Who uses the key: `human` (you), `agent` (an AI agent: risky commands wait for a person's
    /// approval) or `service` (another program). The audit log records it as `actor_kind`.
    #[schema(example = "agent")]
    kind: String,
}

impl From<ApiKey> for ApiKeyResponse {
    fn from(key: ApiKey) -> Self {
        ApiKeyResponse {
            id: key.id,
            name: key.name,
            prefix: key.prefix,
            permissions: key.permissions,
            org_id: key.org_id,
            expires_at: key.expires_at,
            allowed_ips: key.allowed_ips,
            rotated_from: key.rotated_from,
            status: key.status,
            approved_by: key.approved_by,
            approved_at: key.approved_at,
            last_used_at: key.last_used_at,
            created_at: key.created_at,
            kind: crate::api_key::KeyKind::key_name(key.kind).to_owned(),
        }
    }
}

/// Approve an API key that waits for approval.
///
/// Requires `auth.api_keys.approve`, from a signed-in session (an API key cannot approve keys).
/// The key works from now on; the audit log records who approved it. To refuse a key, revoke it
/// with `DELETE /api/v1/auth/all-api-keys/{id}`.
#[utoipa::path(
    post,
    path = "/all-api-keys/{id}/approve",
    extensions(("x-permissions" = json!(["auth.api_keys.approve"]))),
    operation_id = "approve_api_key",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the key.")),
    responses(
        (status = 200, description = "The key, now `active`.", body = Data<ApiKeyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.api_keys.approve` (code `permission_denied`), or the request used an API key (code `auth.api_key_cannot_approve_keys`).", body = Problem),
        (status = 404, description = "No key has this id; code `auth.api_key_not_found`.", body = Problem),
        (status = 409, description = "The key does not wait for approval; code `auth.api_key_not_pending`.", body = Problem),
    )
)]
async fn approve_api_key(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ApiKeyResponse>, Error> {
    let key = state.api_keys.approve(&tenant, &principal, id).await?;
    Ok(Data { data: key.into() })
}

/// A new API key, with its secret. The secret is shown only here.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct CreatedApiKeyResponse {
    /// The key. Send it as `Authorization: Bearer <key>`; no other header is needed: the key
    /// names its tenant. Store it now: it cannot be shown again. Shape: `axk1_`, the tenant ID
    /// (32 lowercase hex digits), `_`, then a random part. Keys created in older versions start
    /// with `axk_` and keep working.
    #[schema(example = "axk1_0193c5c0d2f87b2a9e4f6a1b2c3d4e5f_3q9ZbXcV…")]
    key: String,
    #[serde(flatten)]
    details: ApiKeyResponse,
}

/// A key to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateApiKeyRequest {
    /// What the key is for, 1 to 100 characters.
    #[schema(example = "nightly report agent")]
    name: String,
    /// Who will use the key: `human` (you, the default), `agent` (an AI agent: commands marked
    /// for approval wait for a person to approve them) or `service` (another program).
    #[serde(default)]
    #[schema(example = "agent")]
    kind: Option<String>,
    /// Permissions the key may use: at least one, each one you have yourself (see
    /// `GET /api/v1/auth/me`). Give a key only what its job needs.
    #[schema(example = json!(["auth.users.read"]))]
    permissions: Vec<String>,
    /// Bind the key to an organization unit you work in: it then acts only on data of that unit
    /// and the units below it. Omit for the whole tenant.
    org_id: Option<Uuid>,
    /// When the key stops working (RFC 3339, in the future, at most
    /// `AUTOXIDE_AUTH_API_KEY_MAX_DAYS` days ahead — 365 by default). Omit for that longest
    /// lifetime: every key expires.
    expires_at: Option<Timestamp>,
    /// IP addresses or ranges (`203.0.113.7`, `203.0.113.0/24`, `2001:db8::/32`) the key may be
    /// used from, at most 50. Omit for any address.
    #[serde(default)]
    #[schema(example = json!(["203.0.113.0/24"]))]
    allowed_ips: Vec<String>,
    /// Your current password: a stolen access token alone must not create a key that
    /// outlives it. Or confirm with `totp_code` or a passkey (`passkey_ceremony_id` and
    /// `passkey_credential`) or a `reauth_token` instead — exactly one of them.
    #[serde(default)]
    current_password: String,
    /// A code from your authenticator app, instead of the password (two-step sign-in must be
    /// on). Wrong codes count towards the same limit as at sign-in.
    #[schema(example = "123456")]
    totp_code: Option<String>,
    /// One of your passkeys, instead of the password: start with
    /// `POST /api/v1/auth/login/passkey/options`, let the browser sign its `options`, and send the
    /// `ceremony_id` here with the browser's answer in `passkey_credential`.
    passkey_ceremony_id: Option<Uuid>,
    /// The browser's `AuthenticationResponseJSON` (from `navigator.credentials.get`) for
    /// `passkey_ceremony_id`.
    #[schema(value_type = Option<Object>)]
    passkey_credential: Option<serde_json::Value>,
    /// A confirmation token from `POST /api/v1/auth/me/reauth`, instead of the password.
    reauth_token: Option<String>,
}

impl From<CreateApiKeyRequest> for NewApiKey {
    fn from(input: CreateApiKeyRequest) -> Self {
        NewApiKey {
            kind: crate::api_key::KeyKind::of_key(input.kind.as_deref().unwrap_or("human")),
            name: input.name,
            permissions: input.permissions,
            org_id: input.org_id,
            expires_at: input.expires_at,
            allowed_ips: input.allowed_ips,
            current_password: input.current_password,
            second_factor: match (input.totp_code, input.passkey_ceremony_id) {
                _ if input.reauth_token.is_some() => input.reauth_token.map(Reauth::Token),
                (Some(code), _) => Some(Reauth::Totp(code)),
                (None, Some(ceremony_id)) => Some(Reauth::Passkey {
                    ceremony_id,
                    // Thiếu câu trả lời thì kiểm passkey báo `credential` không hợp lệ.
                    credential: input.passkey_credential.unwrap_or_default(),
                }),
                (None, None) => None,
            },
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
/// Any signed-in user. Newest first unless `order=asc`. Secrets are never listed. Called with
/// an API key, the list holds only that key; called with an OAuth token, it is empty.
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
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<ApiKeyResponse>, Error> {
    let page = state.api_keys.list_own(&tenant, &principal, &PageRequest::try_from(query)?).await?;
    Ok(page.map(ApiKeyResponse::from))
}

/// Create an API key for a program or an AI agent.
///
/// Signed in with a password session only: an API key cannot create keys. The key
/// acts as you, limited to the permissions listed. Confirm it is you with your current
/// password, a code from your authenticator app (`totp_code`) or one of your passkeys
/// (`passkey_ceremony_id`, `passkey_credential`). The response carries the key itself, shown only once.
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
        (status = 400, description = "The current password is wrong; code `auth.wrong_password`.", body = Problem),
        (status = 401, description = "The passkey is not one of yours or its answer is not valid; code `auth.passkey_sign_in_failed`.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.api_key_cannot_create_keys`.", body = Problem),
        (status = 409, description = "You have too many keys (code `auth.api_key_limit`), or sent `totp_code` without two-step sign-in on (code `auth.mfa_not_enabled`).", body = Problem),
        (status = 422, description = "A value is invalid, e.g. a permission you do not have (`not_granted`), a wrong `totp_code`, or both a password and a second factor; see `errors`.", body = Problem),
        (status = 429, description = "Too many wrong passwords or codes; code `auth.login_throttled`. Wait the number of seconds in the `Retry-After` header.", body = Problem),
    )
)]
async fn create_api_key(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateApiKeyRequest>,
) -> Result<Created<CreatedApiKeyResponse>, Error> {
    if input.kind.as_deref().is_some_and(|kind| !["human", "agent", "service"].contains(&kind)) {
        return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
            "kind",
            "invalid",
            "must be `human`, `agent` or `service`",
        )]));
    }
    let created = state.api_keys.create(&tenant, &principal, input.into()).await?;
    Ok(Created::new(format!("/auth/api-keys/{}", created.key.id), created.into()))
}

/// Revoke one of your API keys.
///
/// The key stops working at once. Called with an API key, only that same key can be revoked (a
/// leaked key can switch itself off); any other id answers `404`. An OAuth token revokes no key
/// (`404`).
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
        (status = 404, description = "You have no key with this id, or the request used another API key; code `auth.api_key_not_found`.", body = Problem),
    )
)]
async fn revoke_api_key(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    state.api_keys.revoke(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
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
            Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
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
            assert!(key.starts_with("axk1_"), "{key}");
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

            // Key khai dải IP: kernel kiểm địa chỉ của request trước mọi handler (bước 54). Request
            // trong test không có địa chỉ client, nên bị từ chối.
            let fenced = json!({"name": "office", "permissions": ["auth.users.read"], "current_password": PASSWORD, "allowed_ips": ["203.0.113.0/24"]});
            let (status, _, made) = call(&app, "POST", "/api-keys", Some(&session), Some(fenced)).await;
            assert_eq!(status, StatusCode::CREATED, "{made}");
            assert_eq!(made["data"]["allowed_ips"], json!(["203.0.113.0/24"]));
            assert!(made["data"]["expires_at"].is_string(), "mọi key có hạn");
            let office = format!("Bearer {}", made["data"]["key"].as_str().unwrap());
            let (status, _, refused) = call(&app, "GET", "/users", Some(&office), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(refused["code"], "network_not_allowed");
            let office_path = format!("/api-keys/{}", made["data"]["id"].as_str().unwrap());
            let (status, _, _) = call(&app, "DELETE", &office_path, Some(&session), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);

            let (status, _, list) = call(&app, "GET", "/api-keys", Some(&session), None).await;
            assert_eq!(status, StatusCode::OK, "{list}");
            assert!(list["data"][0].get("key").is_none());
            // `axk1_` + ID khách (32 hex) + `_`: phần nhận diện là đầu phần ngẫu nhiên.
            assert_eq!(list["data"][0]["prefix"], json!(format!("axk1_…{}", &key[38..46])));

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

    // Bước 54: khách bật duyệt key thì key của nhân viên chờ tới khi quản trị duyệt; key của người
    // giữ quyền duyệt chạy ngay; duyệt hai lần thì 409.
    #[tokio::test]
    async fn key_cho_quan_tri_duyet() {
        TestDatabase::run(async |test| {
            use autoxide_kernel::event::Actor;
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            let root = Accounts::create(services.users.as_ref(), &tenant, root).await.unwrap();
            let ada = NewAccount {
                email: "eve@example.com".to_owned(),
                name: "Eve".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: false,
            };
            let ada = Accounts::create(services.users.as_ref(), &tenant, ada).await.unwrap();
            let reader = crate::role::RoleInput {
                denies: Vec::new(),
                requires_mfa: false,
                grants: Vec::new(),
                name: "reader".to_owned(),
                description: String::new(),
                permissions: vec!["auth.users.read".to_owned()],
            };
            let reader = services.roles.create(&tenant, Actor::system(), reader).await.unwrap();
            services.roles.set_user_roles(&tenant, Actor::system(), ada.id, vec![reader.id]).await.unwrap();
            let boss = bearer(&app, "root@example.com", PASSWORD).await;
            let staff = bearer(&app, "eve@example.com", PASSWORD).await;

            let answer = app
                .request(axum::http::Method::PUT, "/api/v1/kernel/settings/auth.api_keys.require_approval")
                .header("authorization", &boss)
                .json(json!({"value": "true"}))
                .send()
                .await
                .unwrap();
            assert!(answer.status().is_success(), "{answer:?}");

            let body = json!({"name": "report", "permissions": ["auth.users.read"], "current_password": PASSWORD});
            let (status, _, made) = call(&app, "POST", "/api-keys", Some(&staff), Some(body.clone())).await;
            assert_eq!(status, StatusCode::CREATED, "{made}");
            assert_eq!(made["data"]["status"], "pending");
            let id = made["data"]["id"].as_str().unwrap().to_owned();
            let key = format!("Bearer {}", made["data"]["key"].as_str().unwrap());
            let (status, _, waiting) = call(&app, "GET", "/users", Some(&key), None).await;
            assert_eq!((status, waiting["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("auth.api_key_pending")));

            let (status, _, inbox) = call(&app, "GET", "/all-api-keys?status=pending", Some(&boss), None).await;
            assert_eq!(status, StatusCode::OK, "{inbox}");
            assert_eq!(inbox["data"].as_array().unwrap().len(), 1);
            assert_eq!(inbox["data"][0]["id"], json!(id));
            let (status, _, _) = call(&app, "GET", "/all-api-keys?status=nope", Some(&boss), None).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            // Nhân viên không duyệt được.
            let approve = format!("/all-api-keys/{id}/approve");
            let (status, _, _) = call(&app, "POST", &approve, Some(&staff), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);

            let (status, _, approved) = call(&app, "POST", &approve, Some(&boss), None).await;
            assert_eq!(status, StatusCode::OK, "{approved}");
            assert_eq!(approved["data"]["status"], "active");
            assert_eq!(approved["data"]["approved_by"], json!(root.id));
            assert!(approved["data"]["approved_at"].is_string());
            let (status, _, users) = call(&app, "GET", "/users", Some(&key), None).await;
            assert_eq!(status, StatusCode::OK, "{users}");
            let (status, _, again) = call(&app, "POST", &approve, Some(&boss), None).await;
            assert_eq!((status, again["code"].as_str()), (StatusCode::CONFLICT, Some("auth.api_key_not_pending")));

            // Người giữ quyền duyệt: key chạy ngay.
            let (status, _, own) = call(&app, "POST", "/api-keys", Some(&boss), Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{own}");
            assert_eq!(own["data"]["status"], "active");
            assert!(own["data"]["approved_by"].is_null());
        })
        .await
        .unwrap();
    }
}
