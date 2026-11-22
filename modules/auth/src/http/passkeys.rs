//! Endpoint passkey: người dùng đã đăng nhập thêm / xem / xoá passkey; đăng nhập bằng
//! passkey. Mỗi nghi thức là hai request: lấy `options` (đưa cho
//! `navigator.credentials.create()` / `.get()` của trình duyệt) rồi gửi lại câu trả lời
//! của authenticator kèm `ceremony_id`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::session::TokenResponse;
use super::{HttpState, require_session};
use crate::passkey::{Ceremony, Passkey};
use crate::permissions::PASSKEYS_MANAGE;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(start_registration))
        .routes(routes!(list_passkeys, add_passkey))
        .routes(routes!(delete_passkey))
        .routes(routes!(list_user_passkeys, remove_user_passkeys))
        .routes(routes!(remove_user_passkey))
}

/// Đăng nhập bằng passkey: vẫn được khi khách chỉ đọc (A-3.11).
pub(super) fn login_routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(start_login))
        .routes(routes!(login))
        .layer(autoxide_kernel::tenancy::allowed_while_read_only())
}

/// A passkey ceremony in progress.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct CeremonyResponse {
    /// Send this back with the authenticator's answer, within 5 minutes. Works once.
    ceremony_id: Uuid,
    /// WebAuthn options as JSON: pass them to
    /// `PublicKeyCredential.parseCreationOptionsFromJSON()` (adding a passkey) or
    /// `PublicKeyCredential.parseRequestOptionsFromJSON()` (signing in), then to
    /// `navigator.credentials.create()` / `.get()`.
    #[schema(value_type = Object)]
    options: Value,
}

impl From<Ceremony> for CeremonyResponse {
    fn from(ceremony: Ceremony) -> Self {
        CeremonyResponse { ceremony_id: ceremony.id, options: ceremony.options }
    }
}

/// A passkey of the signed-in user. Holds no secret.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct PasskeyResponse {
    id: Uuid,
    /// Name the user gave it, e.g. the device.
    #[schema(example = "Ada's phone")]
    name: String,
    created_at: Timestamp,
    /// When it was last used to sign in; `null` if never.
    last_used_at: Option<Timestamp>,
    /// Kind of authenticator, as it identified itself when registered (its AAGUID); `null` when
    /// it did not say (or registered before this was kept).
    aaguid: Option<Uuid>,
    /// Name of that kind of authenticator, e.g. `Windows Hello`, `Apple Passwords`,
    /// `Google Password Manager`; `null` when the kind is not known to the server.
    #[schema(example = "Windows Hello")]
    authenticator: Option<String>,
}

impl From<Passkey> for PasskeyResponse {
    fn from(passkey: Passkey) -> Self {
        PasskeyResponse {
            id: passkey.id,
            name: passkey.name,
            created_at: passkey.created_at,
            last_used_at: passkey.last_used_at,
            authenticator: passkey
                .aaguid
                .and_then(crate::passkey::authenticators::name)
                .map(str::to_owned),
            aaguid: passkey.aaguid,
        }
    }
}

/// Proof that you are the account owner.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartPasskeyRegistrationRequest {
    /// Your current password: a stolen access token alone must not add a passkey.
    #[serde(default)]
    #[validate(length(max = 1024))]
    current_password: String,
    /// A confirmation token from `POST /api/v1/auth/me/reauth`, instead of `current_password`
    /// (accounts without a password use this). Exactly one of the two.
    reauth_token: Option<String>,
}

/// The authenticator's answer to an add-passkey ceremony.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddPasskeyRequest {
    /// The `ceremony_id` from `POST /api/v1/auth/me/passkeys/registration`.
    ceremony_id: Uuid,
    /// A name to recognize the passkey by, 1 to 100 characters.
    #[schema(example = "Ada's phone")]
    name: String,
    /// The created credential as JSON (`credential.toJSON()` in the browser).
    #[schema(value_type = Object)]
    credential: Value,
}

/// The authenticator's answer to a sign-in ceremony.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PasskeyLoginRequest {
    /// The `ceremony_id` from `POST /api/v1/auth/login/passkey/options`.
    ceremony_id: Uuid,
    /// The assertion as JSON (`credential.toJSON()` in the browser).
    #[schema(value_type = Object)]
    credential: Value,
}

/// Start adding a passkey to your account.
///
/// Needs your current password and a session signed in with a password (an API key cannot do this). Answers 403
/// with code `auth.passkeys_disabled` when the server has passkeys off.
#[utoipa::path(
    post,
    path = "/me/passkeys/registration",
    extensions(("x-permissions" = json!([]))),
    operation_id = "start_passkey_registration",
    security(("bearer" = [])),
    request_body = StartPasskeyRegistrationRequest,
    responses(
        (status = 200, description = "Options for the browser, and the ceremony to finish with `POST /api/v1/auth/me/passkeys`.", body = Data<CeremonyResponse>),
        (status = 400, description = "The current password is wrong; code `auth.wrong_password`. A user who forgot it resets it with `POST /api/v1/auth/password-reset`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Called with an API key (`auth.session_required`), or passkeys are off (`auth.passkeys_disabled`).", body = Problem),
        (status = 409, description = "You already have the most passkeys allowed; code `auth.passkey_limit`.", body = Problem),
    )
)]
async fn start_registration(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<StartPasskeyRegistrationRequest>,
) -> Result<Data<CeremonyResponse>, Error> {
    require_session(&principal)?;
    let ceremony = state
        .passkeys
        .start_registration(
            &tenant,
            principal.user_id(),
            super::confirm(&input.current_password, input.reauth_token.as_deref())?,
        )
        .await?;
    Ok(Data { data: ceremony.into() })
}

/// Add a passkey: finish the ceremony started with `POST /api/v1/auth/me/passkeys/registration`.
///
/// Needs a session signed in with a password. Afterwards the passkey signs in with
/// `POST /api/v1/auth/login/passkey`.
#[utoipa::path(
    post,
    path = "/me/passkeys",
    extensions(("x-permissions" = json!([]))),
    operation_id = "add_passkey",
    security(("bearer" = [])),
    request_body = AddPasskeyRequest,
    responses(
        (status = 201, description = "The passkey was added.", body = Data<PasskeyResponse>),
        (status = 400, description = "The ceremony is unknown, expired or used (`auth.invalid_passkey_ceremony`), or the credential was not accepted (`auth.passkey_rejected`; the message says why).", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Called with an API key (`auth.session_required`), or passkeys are off (`auth.passkeys_disabled`).", body = Problem),
        (status = 409, description = "This passkey is already registered; code `auth.passkey_already_registered`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid; see `errors`.", body = Problem),
    )
)]
async fn add_passkey(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<AddPasskeyRequest>,
) -> Result<Created<PasskeyResponse>, Error> {
    require_session(&principal)?;
    let passkey = state
        .passkeys
        .finish_registration(
            &tenant,
            principal.user_id(),
            input.ceremony_id,
            &input.name,
            &input.credential,
        )
        .await?;
    Ok(Created::new(format!("/auth/me/passkeys/{}", passkey.id), passkey.into()))
}

/// List your passkeys.
///
/// Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/me/passkeys",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_passkeys",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your passkeys.", body = Page<PasskeyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_passkeys(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<PasskeyResponse>, Error> {
    let page =
        state.passkeys.list(&tenant, principal.user_id(), &PageRequest::try_from(query)?).await?;
    Ok(page.map(PasskeyResponse::from))
}

/// Remove one of your passkeys.
///
/// Needs a session signed in with a password. The passkey stops working at once; also
/// delete it from the device.
#[utoipa::path(
    delete,
    path = "/me/passkeys/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "delete_passkey",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the passkey.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Called with an API key; code `auth.session_required`.", body = Problem),
        (status = 404, description = "You have no passkey with this id; code `auth.passkey_not_found`.", body = Problem),
    )
)]
async fn delete_passkey(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    require_session(&principal)?;
    state.passkeys.delete(&tenant, principal.user_id(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Checks for the administrator endpoints: permission, the user exists, and the user has no
/// permission the caller lacks.
async fn check_admin(
    state: &HttpState,
    tenant: &TenantContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), Error> {
    principal.require(&PASSKEYS_MANAGE)?;
    state.users.get(tenant, id).await?.ok_or_else(|| crate::user::user_not_found(id))?;
    state.roles.require_covers(tenant, principal, id).await
}

/// List the passkeys of a user.
///
/// Requires `auth.passkeys.manage`, and that the user has no permission you lack. The same
/// fields as `GET /api/v1/auth/me/passkeys`: no key material. Newest first unless
/// `order=asc`.
#[utoipa::path(
    get,
    path = "/users/{id}/passkeys",
    extensions(("x-permissions" = json!(["auth.passkeys.manage"]))),
    operation_id = "list_user_passkeys",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user."), PageQuery),
    responses(
        (status = 200, description = "One page of the user's passkeys.", body = Page<PasskeyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.passkeys.manage` (code `permission_denied`), or the user has permissions you lack (code `auth.target_has_more_permissions`).", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_user_passkeys(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Result<Page<PasskeyResponse>, Error> {
    let page = PageRequest::try_from(query)?;
    check_admin(&state, &tenant, &principal, id).await?;
    Ok(state.passkeys.list(&tenant, id, &page).await?.map(PasskeyResponse::from))
}

/// Remove one passkey of a user who lost the device.
///
/// Requires `auth.passkeys.manage`, and that the user has no permission you lack. The passkey
/// stops working at once. The password and the user's sessions stay as they are (like
/// `DELETE /api/v1/auth/users/{id}/mfa`). Check who is asking before using this. The event
/// `auth.passkey_removed` records `by_administrator`.
#[utoipa::path(
    delete,
    path = "/users/{id}/passkeys/{passkey_id}",
    extensions(("x-permissions" = json!(["auth.passkeys.manage"]))),
    operation_id = "remove_user_passkey",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user."), ("passkey_id" = Uuid, Path, description = "Id of the passkey.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.passkeys.manage` (code `permission_denied`), or the user has permissions you lack (code `auth.target_has_more_permissions`).", body = Problem),
        (status = 404, description = "No user has this id (code `auth.user_not_found`), or the user has no passkey with this id (code `auth.passkey_not_found`).", body = Problem),
    )
)]
async fn remove_user_passkey(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, passkey_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, Error> {
    check_admin(&state, &tenant, &principal, id).await?;
    state.passkeys.remove_for(&tenant, Actor::from(&principal), id, passkey_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove every passkey of a user who lost their devices.
///
/// Requires `auth.passkeys.manage`, and that the user has no permission you lack. The user
/// signs in with the password and can add new passkeys. The password and sessions stay as they
/// are. Answers 204 also when the user had none.
#[utoipa::path(
    delete,
    path = "/users/{id}/passkeys",
    extensions(("x-permissions" = json!(["auth.passkeys.manage"]))),
    operation_id = "remove_user_passkeys",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    responses(
        (status = 204, description = "Every passkey of the user is removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.passkeys.manage` (code `permission_denied`), or the user has permissions you lack (code `auth.target_has_more_permissions`).", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
    )
)]
async fn remove_user_passkeys(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    check_admin(&state, &tenant, &principal, id).await?;
    state.passkeys.remove_all_for(&tenant, Actor::from(&principal), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Start signing in with a passkey.
///
/// Needs no email: the browser lets the user pick one of their passkeys for this site.
#[utoipa::path(
    post,
    path = "/login/passkey/options",
    extensions(("x-public" = json!(true))),
    operation_id = "start_passkey_login",
    responses(
        (status = 200, description = "Options for the browser, and the ceremony to finish with `POST /api/v1/auth/login/passkey`.", body = Data<CeremonyResponse>),
        (status = 403, description = "Passkeys are off on this server; code `auth.passkeys_disabled`.", body = Problem),
    )
)]
async fn start_login(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
) -> Result<Data<CeremonyResponse>, Error> {
    Ok(Data { data: state.passkeys.start_login(&tenant).await?.into() })
}

/// Sign in with a passkey: finish the ceremony from `POST /api/v1/auth/login/passkey/options`.
///
/// Returns the same tokens as `POST /api/v1/auth/login`. A passkey verifies the user
/// itself, so no two-step code is asked for.
#[utoipa::path(
    post,
    path = "/login/passkey",
    extensions(("x-public" = json!(true))),
    operation_id = "passkey_login",
    request_body = PasskeyLoginRequest,
    responses(
        (status = 200, description = "Signed in.", body = Data<TokenResponse>),
        (status = 400, description = "The ceremony is unknown, expired or used; code `auth.invalid_passkey_ceremony`.", body = Problem),
        (status = 401, description = "The passkey was not accepted; code `auth.passkey_sign_in_failed`.", body = Problem),
        (status = 403, description = "The account is disabled (`auth.user_disabled`), or passkeys are off (`auth.passkeys_disabled`).", body = Problem),
        (status = 422, description = "The body is missing a field, or the credential is not a WebAuthn response; see `errors`.", body = Problem),
    )
)]
async fn login(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    headers: axum::http::HeaderMap,
    ValidJson(input): ValidJson<PasskeyLoginRequest>,
) -> Result<Data<TokenResponse>, Error> {
    let user_agent = super::user_agent(&headers);
    let tokens = state
        .passkeys
        .finish_login(&tenant, input.ceremony_id, &input.credential, user_agent)
        .await?;
    super::note_country(&state, &tenant, &headers, tokens.session_id).await;
    Ok(Data { data: tokens.into() })
}

#[cfg(test)]
mod tests {
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::http::test_support::{PASSWORD, app, bearer, call};
    use crate::passkey::testing::SoftAuthenticator;

    #[tokio::test]
    async fn them_passkey_roi_dang_nhap_qua_api() {
        TestDatabase::run(async |test| {
            let (app, _) = app(test).await;
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;
            let mut phone = SoftAuthenticator::new(1);

            let (status, _, started) = call(
                &app,
                "POST",
                "/me/passkeys/registration",
                Some(&ada),
                Some(json!({"current_password": PASSWORD})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{started}");
            let credential = phone.register(&started["data"]["options"]);
            let body = json!({
                "ceremony_id": started["data"]["ceremony_id"],
                "name": "Ada's phone",
                "credential": credential,
            });
            let (status, _, added) =
                call(&app, "POST", "/me/passkeys", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{added}");
            assert_eq!(added["data"]["name"], "Ada's phone");
            let (_, _, listed) = call(&app, "GET", "/me/passkeys", Some(&ada), None).await;
            assert_eq!(listed["data"].as_array().unwrap().len(), 1);

            // Đăng nhập không cần email, không cần mật khẩu.
            let (status, _, options) =
                call(&app, "POST", "/login/passkey/options", None, None).await;
            assert_eq!(status, StatusCode::OK, "{options}");
            let assertion = phone.sign_in(&options["data"]["options"]);
            let body =
                json!({"ceremony_id": options["data"]["ceremony_id"], "credential": assertion});
            let (status, _, signed_in) =
                call(&app, "POST", "/login/passkey", None, Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{signed_in}");
            let token = format!("Bearer {}", signed_in["data"]["access_token"].as_str().unwrap());
            let (status, _, me) = call(&app, "GET", "/me", Some(&token), None).await;
            assert_eq!(
                (status, me["data"]["email"].as_str()),
                (StatusCode::OK, Some("ada@example.com"))
            );

            let id = added["data"]["id"].as_str().unwrap();
            let (status, _, _) =
                call(&app, "DELETE", &format!("/me/passkeys/{id}"), Some(&ada), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn api_key_khong_them_duoc_passkey() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = crate::api::NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            crate::api::Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let body = json!({"name": "agent", "permissions": ["auth.users.read"], "current_password": PASSWORD});
            let (status, _, key) = call(&app, "POST", "/api-keys", Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{key}");
            let key = format!("Bearer {}", key["data"]["key"].as_str().unwrap());

            let (status, _, problem) =
                call(&app, "POST", "/me/passkeys/registration", Some(&key), Some(json!({"current_password": PASSWORD}))).await;

            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(problem["code"], "auth.session_required");
        })
        .await
        .unwrap();
    }

    /// Quản trị viên có `auth.passkeys.manage` xem và gỡ passkey của người khác (từng cái hay
    /// tất cả), không tác động lên người quyền cao hơn mình; event ghi `by_administrator`.
    #[tokio::test]
    async fn quan_tri_vien_xem_va_go_passkey_cua_nguoi_khac() {
        use autoxide_kernel::event::Actor;

        use crate::api::{Accounts, NewAccount};

        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let account = |email: &str, admin: bool| NewAccount {
                email: email.to_owned(),
                name: "Someone".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin,
            };
            let root = Accounts::create(
                services.users.as_ref(),
                &services.tenant,
                account("root@example.com", true),
            )
            .await
            .unwrap();
            let bob = Accounts::create(
                services.users.as_ref(),
                &services.tenant,
                account("bob@example.com", false),
            )
            .await
            .unwrap();
            for (user, seed) in [(bob.id, 1), (bob.id, 2), (root.id, 3)] {
                let mut device = SoftAuthenticator::new(seed);
                let ceremony =
                    services.passkeys.start_registration(&tenant, user, PASSWORD).await.unwrap();
                let credential = device.register(&ceremony.options);
                services
                    .passkeys
                    .finish_registration(&tenant, user, ceremony.id, "Phone", &credential)
                    .await
                    .unwrap();
            }
            let helpdesk = crate::role::RoleInput {
                denies: Vec::new(),
                requires_mfa: false,
                grants: Vec::new(),
                name: "helpdesk".to_owned(),
                description: String::new(),
                permissions: vec!["auth.passkeys.manage".to_owned()],
            };
            let helpdesk = services.roles.create(&tenant, Actor::system(), helpdesk).await.unwrap();
            let ada =
                services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
            services
                .roles
                .set_user_roles(&tenant, Actor::system(), ada.id, vec![helpdesk.id])
                .await
                .unwrap();
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;
            let bob_token = bearer(&app, "bob@example.com", PASSWORD).await;

            // Không có permission: 403.
            let list = format!("/users/{}/passkeys", bob.id);
            let (status, _, problem) = call(&app, "GET", &list, Some(&bob_token), None).await;
            assert_eq!(
                (status, problem["code"].as_str()),
                (StatusCode::FORBIDDEN, Some("permission_denied"))
            );
            // Người quyền cao hơn: 403.
            for (method, path) in [
                ("GET", format!("/users/{}/passkeys", root.id)),
                ("DELETE", format!("/users/{}/passkeys", root.id)),
            ] {
                let (status, _, problem) = call(&app, method, &path, Some(&ada), None).await;
                assert_eq!(
                    (status, problem["code"].as_str()),
                    (StatusCode::FORBIDDEN, Some("auth.target_has_more_permissions")),
                    "{method} {path}"
                );
            }

            let (status, _, page) = call(&app, "GET", &list, Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            let ids: Vec<String> = page["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(ids.len(), 2);
            let one = format!("/users/{}/passkeys/{}", bob.id, ids[0]);
            let (status, _, _) = call(&app, "DELETE", &one, Some(&ada), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let (status, _, problem) = call(&app, "DELETE", &one, Some(&ada), None).await;
            assert_eq!(
                (status, problem["code"].as_str()),
                (StatusCode::NOT_FOUND, Some("auth.passkey_not_found"))
            );
            let (status, _, _) = call(&app, "DELETE", &list, Some(&ada), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let (_, _, page) = call(&app, "GET", &list, Some(&ada), None).await;
            assert_eq!(page["data"], json!([]));
            let missing = format!("/users/{}/passkeys", uuid::Uuid::now_v7());
            let (status, _, problem) = call(&app, "DELETE", &missing, Some(&ada), None).await;
            assert_eq!(
                (status, problem["code"].as_str()),
                (StatusCode::NOT_FOUND, Some("auth.user_not_found"))
            );

            // Hai event `auth.passkey_removed`, đều do quản trị viên.
            use sea_orm::ConnectionTrait as _;
            let backend = services.database.read().get_database_backend();
            let rows = services
                .database
                .read()
                .query_all_raw(sea_orm::Statement::from_string(
                    backend,
                    "SELECT payload FROM auth_events WHERE name = 'auth.passkey_removed'",
                ))
                .await
                .unwrap();
            assert_eq!(rows.len(), 2);
            for row in rows {
                let payload: String = row.try_get("", "payload").unwrap();
                assert!(payload.contains("\"by_administrator\":true"), "{payload}");
            }
        })
        .await
        .unwrap();
    }
}
