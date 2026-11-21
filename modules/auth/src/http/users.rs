//! Endpoint người dùng: quản trị viên tạo và quản lý tài khoản; người lạ tự đăng ký
//! khi được bật.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::api::User;
use crate::permissions::{USERS_MANAGE, USERS_READ};
use crate::role::RoleService;
use crate::user::{AccountFacts, NewUser, UserChanges, UserService, UserStatus};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_users, create_user))
        .routes(routes!(get_user, update_user, delete_user))
        .routes(routes!(register))
        .routes(routes!(request_registration))
        .routes(routes!(set_user_password))
}

/// The first password of an account.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetPasswordRequest {
    /// 12 to 128 characters.
    password: String,
}

/// A user account. Never includes the password.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct UserResponse {
    id: Uuid,
    /// Normalized email: lower case, no surrounding spaces. Unique.
    #[schema(example = "ada@example.com")]
    email: String,
    /// Display name.
    #[schema(example = "Ada Lovelace")]
    name: String,
    /// When the account was disabled; `null` while it is active. A disabled account
    /// cannot sign in or use its tokens.
    disabled_at: Option<Timestamp>,
    /// When the owner confirmed `email` through a verification mail; `null` while it
    /// is not verified.
    email_verified_at: Option<Timestamp>,
    /// Whether the account has a password. Without one it signs in with a passkey; its owner
    /// sets a first password through `POST /api/v1/auth/password-reset`, an administrator
    /// with `POST /api/v1/auth/users/{id}/password`.
    has_password: bool,
    /// Whether two-step sign-in (an authenticator app) is on.
    mfa_enabled: bool,
    /// When the user last started a session (sign-in, password change); `null` if not since
    /// this field exists.
    last_sign_in_at: Option<Timestamp>,
    /// The user's roles, by name. Only with `include=roles` on the user list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    roles: Option<Vec<UserRole>>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

/// A role a user holds.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct UserRole {
    id: Uuid,
    #[schema(example = "editor")]
    name: String,
}

impl UserResponse {
    fn new(user: User, facts: AccountFacts, roles: Option<Vec<UserRole>>) -> Self {
        UserResponse {
            id: user.id,
            email: user.email,
            name: user.name,
            disabled_at: user.disabled_at,
            email_verified_at: user.email_verified_at,
            has_password: facts.has_password,
            mfa_enabled: facts.mfa_enabled,
            last_sign_in_at: facts.last_sign_in_at,
            roles,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

/// Dựng câu trả lời cho một trang người dùng: dữ kiện tài khoản (và role khi `with_roles`)
/// lấy theo lô cho cả trang — không gọi một lần cho mỗi người (đề bài giao diện B1).
pub(crate) async fn describe_page(
    users: &UserService,
    roles: &RoleService,
    tenant: &TenantContext,
    page: Page<User>,
    with_roles: bool,
) -> Result<Page<UserResponse>, Error> {
    let ids: Vec<Uuid> = page.data.iter().map(|user| user.id).collect();
    let facts = users.facts(tenant, &ids).await?;
    let mut names =
        if with_roles { roles.names_of_users(tenant, &ids).await? } else { Default::default() };
    Ok(page.map(|user| {
        let found = facts.get(&user.id).copied().unwrap_or_default();
        let held = with_roles.then(|| {
            let held = names.remove(&user.id).unwrap_or_default();
            held.into_iter().map(|(id, name)| UserRole { id, name }).collect()
        });
        UserResponse::new(user, found, held)
    }))
}

/// Như [`describe_page`], cho một tài khoản (không kèm role).
pub(crate) async fn describe(
    users: &UserService,
    tenant: &TenantContext,
    user: User,
) -> Result<UserResponse, Error> {
    let facts = users.facts(tenant, &[user.id]).await?.remove(&user.id).unwrap_or_default();
    Ok(UserResponse::new(user, facts, None))
}

/// A new account.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateUserRequest {
    /// Email; also the sign-in name. Case and surrounding spaces do not matter.
    #[schema(example = "ada@example.com")]
    email: String,
    /// Display name, 1 to 200 characters.
    #[schema(example = "Ada Lovelace")]
    name: String,
    /// 12 to 128 characters. When omitted, the account cannot sign in with a password
    /// until one is set.
    password: Option<String>,
}

/// A new account created by its owner.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegisterRequest {
    /// Email; also the sign-in name.
    #[schema(example = "ada@example.com")]
    email: String,
    /// Display name, 1 to 200 characters.
    #[schema(example = "Ada Lovelace")]
    name: String,
    /// 12 to 128 characters.
    password: String,
    /// `staff` (default; needs self-registration on the server) or `customer`: a customer
    /// account of this tenant's shop, without any permission (open unless the tenant turns off
    /// the setting `auth.customer_registration`).
    #[serde(default)]
    realm: autoxide_kernel::access::Realm,
}

/// What `POST /api/v1/auth/registrations` accepted. The same whether the email already had an
/// account or not.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct RegistrationResponse {
    /// The email, normalized. A mail went to it: either a link to confirm the new account, or,
    /// when it already had one, a notice to sign in or reset the password.
    #[schema(example = "ada@example.com")]
    email: String,
}

/// Changes to an account. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateUserRequest {
    /// New display name, 1 to 200 characters.
    name: Option<String>,
    /// `true` disables the account and signs it out everywhere; `false` enables it.
    disabled: Option<bool>,
}

/// Which users to list.
#[derive(Debug, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct UserSearchQuery {
    /// Only users whose email or name contains this text, in any case (`ada` finds
    /// `Ada Lovelace` and `ada@example.com`). Up to 200 characters.
    #[validate(length(max = 200))]
    search: Option<String>,
    /// Only `active` or only `disabled` accounts.
    #[param(inline)]
    status: Option<UserStatusFilter>,
    /// `roles`: add each user's roles (`roles` in each item), in the same request.
    #[param(inline)]
    include: Option<UserInclude>,
}

/// Account state to filter on.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UserStatusFilter {
    Active,
    Disabled,
}

/// Extra data to add to each user of the list.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UserInclude {
    Roles,
}

impl From<UserStatusFilter> for UserStatus {
    fn from(filter: UserStatusFilter) -> Self {
        match filter {
            UserStatusFilter::Active => UserStatus::Active,
            UserStatusFilter::Disabled => UserStatus::Disabled,
        }
    }
}

/// List or search user accounts.
///
/// Requires `auth.users.read`. Newest first unless `order=asc`. Add `search` to find
/// users by email or name, `status` to keep active or disabled ones, `include=roles` to get
/// each user's roles in the same answer.
#[utoipa::path(
    get,
    path = "/users",
    extensions(("x-permissions" = json!(["auth.users.read"]))),
    operation_id = "list_users",
    security(("bearer" = [])),
    params(UserSearchQuery, PageQuery),
    responses(
        (status = 200, description = "One page of users.", body = Page<UserResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_users(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(search): Query<UserSearchQuery>,
    Query(query): Query<PageQuery>,
) -> Result<Page<UserResponse>, Error> {
    principal.require(&USERS_READ)?;
    search.validate().map_err(Error::from)?;
    let status = search.status.map(UserStatus::from);
    let page = PageRequest::try_from(query)?;
    let page = state.users.list(&tenant, search.search.as_deref(), status, &page).await?;
    let with_roles = matches!(search.include, Some(UserInclude::Roles));
    describe_page(&state.users, &state.roles, &tenant, page, with_roles).await
}

/// Create a user account.
///
/// Requires `auth.users.manage`. The account has no role; give it roles with
/// `PUT /api/v1/auth/users/{id}/roles`. A mail asking to confirm the address goes to
/// the new email, in the language of `Accept-Language`.
#[utoipa::path(
    post,
    path = "/users",
    extensions(("x-permissions" = json!(["auth.users.manage"]))),
    operation_id = "create_user",
    security(("bearer" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "The account was created.", body = Data<UserResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "An account has this email; code `auth.email_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_user(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    ValidJson(input): ValidJson<CreateUserRequest>,
) -> Result<Created<UserResponse>, Error> {
    principal.require(&USERS_MANAGE)?;
    let input = NewUser { email: input.email, name: input.name, password: input.password };
    let actor = Actor::from(&principal);
    let user = state.users.create(&tenant, actor, input).await?;
    state.verifications.after_signup(&tenant, actor, &user, language).await;
    let path = format!("/auth/users/{}", user.id);
    Ok(Created::new(path, describe(&state.users, &tenant, user).await?))
}

/// Get a user account.
///
/// Requires `auth.users.read`.
#[utoipa::path(
    get,
    path = "/users/{id}",
    extensions(("x-permissions" = json!(["auth.users.read"]))),
    operation_id = "get_user",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    responses(
        (status = 200, description = "The account.", body = Data<UserResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
    )
)]
async fn get_user(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<UserResponse>, Error> {
    principal.require(&USERS_READ)?;
    let user = state
        .users
        .get(&tenant, id)
        .await?
        .filter(|user| user.realm == autoxide_kernel::access::Realm::Staff)
        .ok_or_else(|| crate::user::user_not_found(id))?;
    Ok(Tagged { version: user.updated_at, data: describe(&state.users, &tenant, user).await? })
}

/// Rename, disable or enable a user account.
///
/// Requires `auth.users.manage`. Disabling signs the user out everywhere: their
/// access tokens stop working at once. The last active user with the `admin` role
/// cannot be disabled.
#[utoipa::path(
    patch,
    path = "/users/{id}",
    extensions(("x-permissions" = json!(["auth.users.manage"]))),
    operation_id = "update_user",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "The account after the change.", body = Data<UserResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.manage` (code `permission_denied`), or the user has permissions you lack (code `auth.target_has_more_permissions`).", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 409, description = "No active user would keep the `admin` role; code `auth.last_admin`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it. Get it again and retry; code `precondition_failed`.", body = Problem),
    )
)]
async fn update_user(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateUserRequest>,
) -> Result<Tagged<UserResponse>, Error> {
    principal.require(&USERS_MANAGE)?;
    state.roles.require_covers(&tenant, &principal, id).await?;
    let changes = UserChanges { name: input.name, disabled: input.disabled, ..Default::default() };
    let user =
        state.users.update(&tenant, Actor::from(&principal), id, changes, if_match.expected()?);
    let user = user.await?;
    Ok(Tagged { version: user.updated_at, data: describe(&state.users, &tenant, user).await? })
}

/// Delete a user account and everything it owns.
///
/// Requires `auth.users.manage`, and every permission the user has. Data kept by other
/// modules is deleted shortly after. This cannot be undone; to only stop someone from
/// signing in, disable the account with `PATCH /api/v1/auth/users/{id}` instead.
#[utoipa::path(
    delete,
    path = "/users/{id}",
    extensions(("x-permissions" = json!(["auth.users.manage"]))),
    operation_id = "delete_user",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    responses(
        (status = 204, description = "The account was deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.manage` (code `permission_denied`), or the user has permissions you lack (code `auth.target_has_more_permissions`).", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 409, description = "This is the last active administrator; code `auth.last_admin`.", body = Problem),
    )
)]
async fn delete_user(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&USERS_MANAGE)?;
    state.roles.require_covers(&tenant, &principal, id).await?;
    state.users.delete(&tenant, Actor::from(&principal), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Create your own account (deprecated: use `POST /api/v1/auth/registrations`).
///
/// Open only when the server enables self-registration; otherwise answers 403. The
/// new account has no role. A mail asking to confirm the address goes to the email,
/// in the language of `Accept-Language`. Then sign in with `POST /api/v1/auth/login`.
#[utoipa::path(
    post,
    path = "/register",
    extensions(("x-public" = json!(true))),
    operation_id = "register",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "The account was created.", body = Data<UserResponse>),
        (status = 403, description = "Self-registration of staff is turned off (code `auth.registration_disabled`), or the shop does not let customers create accounts (code `auth.customer_registration_closed`).", body = Problem),
        (status = 409, description = "An account has this email; code `auth.email_taken`.", body = Problem),
        (status = 422, description = "A value is invalid, e.g. a password shorter than 12 characters; see `errors`.", body = Problem),
    )
)]
async fn register(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    language: Language,
    ValidJson(input): ValidJson<RegisterRequest>,
) -> Result<Created<UserResponse>, Error> {
    let customer = registration_open(&state, &tenant, input.realm).await?;
    let input = NewUser { email: input.email, name: input.name, password: Some(input.password) };
    // Chưa đăng nhập, chưa có tài khoản: "hệ thống" tạo thay người đăng ký.
    let user = if customer {
        state.users.create_customer(&tenant, Actor::system(), input).await?
    } else {
        state.users.create(&tenant, Actor::system(), input).await?
    };
    state.verifications.after_signup(&tenant, Actor::system(), &user, language).await;
    let path = format!("/auth/users/{}", user.id);
    Ok(Created::new(path, describe(&state.users, &tenant, user).await?))
}

/// Create your own account, without telling whether the email already has one.
///
/// Open only when the server enables self-registration (or, for `customer`, when the tenant
/// lets customers create accounts); otherwise answers 403. Always answers 202 with the same
/// body for a valid request: a new account (without any role) gets a mail to confirm the
/// address; an email that already has an account gets a notice instead, and nothing changes.
/// Then sign in with `POST /api/v1/auth/login`. Replaces `POST /api/v1/auth/register`, which
/// tells with 409 that an email is taken.
#[utoipa::path(
    post,
    path = "/registrations",
    extensions(("x-public" = json!(true))),
    operation_id = "request_registration",
    request_body = RegisterRequest,
    responses(
        (status = 202, description = "Accepted; a mail went to the email (confirm the new account, or a notice when it already had one).", body = Data<RegistrationResponse>),
        (status = 403, description = "Self-registration of staff is turned off (code `auth.registration_disabled`), or the shop does not let customers create accounts (code `auth.customer_registration_closed`).", body = Problem),
        (status = 422, description = "A value is invalid, e.g. a password shorter than 12 characters; see `errors`.", body = Problem),
    )
)]
async fn request_registration(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    language: Language,
    ValidJson(input): ValidJson<RegisterRequest>,
) -> Result<(StatusCode, Data<RegistrationResponse>), Error> {
    let customer = registration_open(&state, &tenant, input.realm).await?;
    let realm = input.realm;
    let email = autoxide_kernel::email::normalize(&input.email);
    let input = NewUser { email: input.email, name: input.name, password: Some(input.password) };
    // Kiểm đầu vào và hash mật khẩu xảy ra ở cả hai nhánh (trong `create`): thời gian trả lời
    // không cho biết email đã có tài khoản hay chưa.
    let created = if customer {
        state.users.create_customer(&tenant, Actor::system(), input).await
    } else {
        state.users.create(&tenant, Actor::system(), input).await
    };
    match created {
        Ok(user) => {
            state.verifications.after_signup(&tenant, Actor::system(), &user, language).await;
        }
        Err(error) if error.code() == "auth.email_taken" => {
            if let Some(user) = state.users.find_by_email_in(&tenant, realm, &email).await? {
                state.verifications.notify_existing_registration(&tenant, &user, language).await;
            }
        }
        Err(error) => return Err(error),
    }
    Ok((StatusCode::ACCEPTED, Data { data: RegistrationResponse { email } }))
}

/// `Ok(true)` cho người mua, `Ok(false)` cho nhân sự, khi khách / máy chủ cho tự đăng ký loại
/// tài khoản `realm`; không thì 403.
async fn registration_open(
    state: &HttpState,
    tenant: &TenantContext,
    realm: autoxide_kernel::access::Realm,
) -> Result<bool, Error> {
    let customer = realm == autoxide_kernel::access::Realm::Customer;
    if customer {
        let open = crate::settings::enabled(
            state.orgs.as_ref(),
            tenant,
            &crate::settings::CUSTOMER_REGISTRATION,
        )
        .await?;
        if !open {
            return Err(Error::forbidden(
                "auth.customer_registration_closed",
                "This shop does not let customers create accounts yet.",
            ));
        }
    } else if !state.self_registration {
        return Err(Error::forbidden(
            "auth.registration_disabled",
            "This server does not let people create their own accounts. Ask an administrator \
             to create one for you.",
        ));
    }
    Ok(customer)
}

/// Set the password of an account created without one.
///
/// Requires `auth.users.manage`. Only for an account that has no password yet: an
/// administrator cannot replace someone else's password. Its owner changes it with
/// `POST /api/v1/auth/me/password` or resets it by email.
#[utoipa::path(
    put,
    path = "/users/{id}/password",
    extensions(("x-permissions" = json!(["auth.users.manage"]))),
    operation_id = "set_user_password",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    request_body = SetPasswordRequest,
    responses(
        (status = 204, description = "The password was set; the user can sign in with it."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.manage` (code `permission_denied`), or the user has permissions you lack (code `auth.target_has_more_permissions`).", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 409, description = "The account already has a password; code `auth.password_already_set`. Its owner changes it with `POST /api/v1/auth/me/password` or resets it with `POST /api/v1/auth/password-reset`.", body = Problem),
        (status = 422, description = "The password breaks the rules; see `errors`.", body = Problem),
    )
)]
async fn set_user_password(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SetPasswordRequest>,
) -> Result<StatusCode, Error> {
    principal.require(&USERS_MANAGE)?;
    state.roles.require_covers(&tenant, &principal, id).await?;
    state.users.set_initial_password(&tenant, Actor::from(&principal), id, input.password).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use autoxide_kernel::event::Actor;

    use crate::api::{Accounts, NewAccount};
    use crate::http::test_support::{PASSWORD, app, app_with, bearer, call};

    /// Đọc thì có `ETag`; sửa kèm `If-Match` cũ thì 412, không ghi đè thay đổi của người
    /// khác; không gửi `If-Match` thì sửa như thường (D-084).
    #[tokio::test]
    async fn etag_va_if_match_chong_ghi_de() {
        use crate::http::test_support::call_with;

        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let ada =
                services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
            let path = format!("/users/{}", ada.id);

            let (status, headers, _) = call(&app, "GET", &path, Some(&admin), None).await;
            assert_eq!(status, StatusCode::OK);
            let etag = headers[axum::http::header::ETAG].to_str().unwrap().to_owned();

            let patch = |if_match: String, name: &str| {
                let (admin, path, name) = (admin.clone(), path.clone(), name.to_owned());
                let app = &app;
                async move {
                    let headers =
                        [("authorization", admin.as_str()), ("if-match", if_match.as_str())];
                    call_with(app, "PATCH", &path, &headers, Some(json!({"name": name}))).await
                }
            };
            let (status, headers, body) = patch(etag.clone(), "Ada L").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let newer = headers[axum::http::header::ETAG].to_str().unwrap().to_owned();
            assert_ne!(newer, etag);

            // Người thứ hai còn giữ ETag cũ: bị từ chối, tên không bị đè.
            let (status, _, problem) = patch(etag, "Stale").await;
            assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{problem}");
            assert_eq!(problem["code"], "precondition_failed");
            let (status, _, _) = patch("W/\"garbage\"".to_owned(), "Stale").await;
            assert_eq!(status, StatusCode::PRECONDITION_FAILED);
            let (_, _, current) = call(&app, "GET", &path, Some(&admin), None).await;
            assert_eq!(current["data"]["name"], "Ada L");

            // Không `If-Match`: sửa như cũ.
            let body = json!({"name": "Ada"});
            let (status, _, _) = call(&app, "PATCH", &path, Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::OK);
        })
        .await
        .unwrap();
    }

    /// Mọi lỗi của đầu vào báo trong MỘT lần (tài liệu API hứa vậy): email sai và tên
    /// rỗng cùng có trong `errors`, không phải sửa một lỗi mới thấy lỗi kia.
    #[tokio::test]
    async fn bao_moi_loi_dau_vao_mot_lan() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let fields = |problem: &serde_json::Value| {
                let mut fields: Vec<String> = problem["errors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|error| error["field"].as_str().unwrap().to_owned())
                    .collect();
                fields.sort();
                fields
            };

            let body = json!({"email": "not-an-email", "name": "", "password": "short"});
            let (status, _, problem) = call(&app, "POST", "/users", Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
            assert_eq!(fields(&problem), ["email", "name", "password"]);

            let body = json!({"name": "", "permissions": [], "current_password": ""});
            let (status, _, problem) =
                call(&app, "POST", "/api-keys", Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
            assert_eq!(fields(&problem), ["current_password", "name", "permissions"]);

            let body = json!({"current_password": "", "new_password": "short"});
            let (status, _, problem) =
                call(&app, "POST", "/me/password", Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
            assert_eq!(fields(&problem), ["current_password", "new_password"]);

            let body = json!({"name": "Bad Name!", "description": "x".repeat(501)});
            let (status, _, problem) = call(&app, "POST", "/roles", Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
            assert_eq!(fields(&problem), ["description", "name"]);
        })
        .await
        .unwrap();
    }

    /// Tìm theo email hoặc tên, không phân biệt hoa thường — kể cả chữ có dấu, và cho
    /// cùng kết quả trên hai database; `%` và `_` là chữ thường, không phải ký tự đại diện.
    #[tokio::test]
    async fn tim_nguoi_dung_theo_email_hoac_ten() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
            for (email, name) in [("dat@example.vn", "Trần Đạt"), ("x_y@example.com", "100% Bob")]
            {
                let account = NewAccount {
                    email: email.to_owned(),
                    name: name.to_owned(),
                    password: None,
                    admin: false,
                };
                Accounts::create(services.users.as_ref(), &services.tenant, account).await.unwrap();
            }
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let emails = async |search: &str| {
                let path = format!("/users?order=asc&search={search}");
                let (status, _, page) = call(&app, "GET", &path, Some(&admin), None).await;
                assert_eq!(status, StatusCode::OK, "{page}");
                page["data"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|user| user["email"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            };

            assert_eq!(emails("EXAMPLE.VN").await, ["dat@example.vn"]);
            // "đạt" viết thường, tìm "ĐẠT" viết hoa (đã mã hoá cho URL).
            assert_eq!(emails("%C4%90%E1%BA%A0T").await, ["dat@example.vn"]);
            // Không dấu vẫn ra: "tran dat" tìm ra "Trần Đạt".
            assert_eq!(emails("tran%20dat").await, ["dat@example.vn"]);
            assert_eq!(emails("100%25").await, ["x_y@example.com"]);
            assert_eq!(emails("x_y").await, ["x_y@example.com"]);
            assert!(emails("x%25y").await.is_empty());
            assert!(emails("a_a").await.is_empty());
            assert_eq!(emails("%20").await.len(), 4);

            // Đổi tên thì tìm theo tên mới.
            let dat =
                services.users.find_by_email(&tenant, "dat@example.vn").await.unwrap().unwrap();
            let body = json!({"name": "Grace"});
            let path = format!("/users/{}", dat.id);
            let (status, _, _) = call(&app, "PATCH", &path, Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(emails("grace").await, ["dat@example.vn"]);
            // Tên cũ "Trần Đạt" không còn.
            assert!(emails("tr%E1%BA%A7n").await.is_empty());

            let long = "a".repeat(201);
            let path = format!("/users?search={long}");
            let (status, _, _) = call(&app, "GET", &path, Some(&admin), None).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        })
        .await
        .unwrap();
    }

    /// Người chỉ có `auth.users.manage` không gỡ MFA, không khoá, không đặt mật khẩu
    /// cho người có quyền cao hơn mình; với người ngang hoặc thấp hơn thì được (D-079).
    #[tokio::test]
    async fn khong_tac_dong_len_nguoi_quyen_cao_hon() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: None,
                admin: true,
            };
            let root =
                Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
            let bob = NewAccount {
                email: "bob@example.com".to_owned(),
                name: "Bob".to_owned(),
                password: None,
                admin: false,
            };
            let bob =
                Accounts::create(services.users.as_ref(), &services.tenant, bob).await.unwrap();
            let helpdesk = crate::role::RoleInput {
                denies: Vec::new(),
                requires_mfa: false,
                grants: Vec::new(),
                name: "helpdesk".to_owned(),
                description: String::new(),
                permissions: vec!["auth.users.manage".to_owned()],
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

            let password = json!({"password": "a brand new password"});
            let disable = json!({"disabled": true});
            let requests = [
                ("DELETE", format!("/users/{}/mfa", root.id), None),
                ("PATCH", format!("/users/{}", root.id), Some(disable.clone())),
                ("PUT", format!("/users/{}/password", root.id), Some(password.clone())),
            ];
            for (method, path, body) in requests {
                let (status, _, problem) = call(&app, method, &path, Some(&ada), body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {problem}");
                assert_eq!(problem["code"], "auth.target_has_more_permissions");
            }

            let path = format!("/users/{}/mfa", bob.id);
            let (status, _, _) = call(&app, "DELETE", &path, Some(&ada), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let path = format!("/users/{}/password", bob.id);
            let (status, _, _) = call(&app, "PUT", &path, Some(&ada), Some(password)).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let path = format!("/users/{}", bob.id);
            let (status, _, body) = call(&app, "PATCH", &path, Some(&ada), Some(disable)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn quan_tri_vien_tao_xem_va_khoa_nguoi_dung() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;

            // Người không có quyền không tạo được.
            let body = json!({"email": "bob@example.com", "name": "Bob", "password": PASSWORD});
            let (status, _, _) = call(&app, "POST", "/users", Some(&ada), Some(body.clone())).await;
            assert_eq!(status, StatusCode::FORBIDDEN);

            let (status, headers, created) =
                call(&app, "POST", "/users", Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            let location =
                format!("/api/v1/auth/users/{}", created["data"]["id"].as_str().unwrap());
            assert_eq!(headers[axum::http::header::LOCATION], location.as_str());
            assert_eq!(created["data"]["email"], "bob@example.com");
            assert!(created["data"]["disabled_at"].is_null());
            assert!(created["data"].get("password").is_none());
            let id = created["data"]["id"].as_str().unwrap().to_owned();

            let (status, _, list) = call(&app, "GET", "/users?limit=2", Some(&admin), None).await;
            assert_eq!(status, StatusCode::OK, "{list}");
            assert_eq!(list["data"][0]["email"], "bob@example.com");

            let path = format!("/users/{id}");
            let (status, _, got) = call(&app, "GET", &path, Some(&admin), None).await;
            assert_eq!((status, got["data"]["name"].clone()), (StatusCode::OK, json!("Bob")));

            let bob = bearer(&app, "bob@example.com", PASSWORD).await;
            let body = json!({"disabled": true});
            let (status, _, changed) = call(&app, "PATCH", &path, Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{changed}");
            assert!(changed["data"]["disabled_at"].is_string());
            // Access token của người bị khoá mất hiệu lực ngay.
            let (status, _, problem) = call(&app, "GET", "/me", Some(&bob), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(problem["code"], "auth.user_disabled");

            let missing = format!("/users/{}", uuid::Uuid::now_v7());
            let (status, _, problem) = call(&app, "GET", &missing, Some(&admin), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(problem["code"], "auth.user_not_found");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tu_dang_ky_mac_dinh_tat() {
        TestDatabase::run(async |test| {
            let (app, _) = app(test).await;
            let body = json!({"email": "eve@example.com", "name": "Eve", "password": PASSWORD});

            let (status, _, problem) = call(&app, "POST", "/register", None, Some(body)).await;

            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(problem["code"], "auth.registration_disabled");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tu_dang_ky_khi_bat_roi_dang_nhap_duoc() {
        TestDatabase::run(async |test| {
            let (app, _) = app_with(test, true).await;
            let body = json!({"email": "Eve@Example.com", "name": "Eve", "password": PASSWORD});

            let (status, _, created) =
                call(&app, "POST", "/register", None, Some(body.clone())).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            assert_eq!(created["data"]["email"], "eve@example.com");

            let eve = bearer(&app, "eve@example.com", PASSWORD).await;
            let (_, _, me) = call(&app, "GET", "/me", Some(&eve), None).await;
            assert_eq!(me["data"]["permissions"], json!([]));
            assert_eq!(me["data"]["all_permissions"], false);

            let (status, _, taken) = call(&app, "POST", "/register", None, Some(body)).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(taken["code"], "auth.email_taken");
            let short = json!({"email": "x@example.com", "name": "X", "password": "short"});
            let (status, _, _) = call(&app, "POST", "/register", None, Some(short)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        })
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod event_tests {
    use autoxide_kernel::time::Timestamp;
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::api::{Accounts, NewAccount};
    use crate::http::test_support::{PASSWORD, app, bearer, call};
    use crate::mfa::service::tests::code_at;
    use crate::testing::recorded_events;

    /// Mỗi thay đổi ghi một event, đúng người làm, và không event nào chứa bí mật.
    #[tokio::test]
    async fn moi_thay_doi_ghi_event_khong_lo_bi_mat() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            let root = Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;

            let body = json!({"email": "bob@example.com", "name": "Bob", "password": PASSWORD});
            let (_, _, bob) = call(&app, "POST", "/users", Some(&admin), Some(body)).await;
            let bob_id = bob["data"]["id"].as_str().unwrap().to_owned();
            let body = json!({"name": "reader", "permissions": ["auth.users.read"]});
            let (_, _, role) = call(&app, "POST", "/roles", Some(&admin), Some(body)).await;
            let body = json!({"role_ids": [role["data"]["id"]]});
            let path = format!("/users/{bob_id}/roles");
            let (status, _, _) = call(&app, "PUT", &path, Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::OK);
            let body = json!({"disabled": true});
            let path = format!("/users/{bob_id}");
            call(&app, "PATCH", &path, Some(&admin), Some(body)).await;
            let body = json!({"name": "agent", "permissions": ["auth.users.read"], "current_password": PASSWORD});
            let (_, _, key) = call(&app, "POST", "/api-keys", Some(&admin), Some(body)).await;
            let (_, _, setup) = call(&app, "POST", "/me/mfa/totp", Some(&admin), Some(json!({"current_password": PASSWORD}))).await;
            let secret = setup["data"]["secret"].as_str().unwrap().to_owned();
            let now = u64::try_from(Timestamp::now().as_unix_seconds()).unwrap();
            let body = json!({"code": code_at(&secret, now)});
            let (_, _, codes) =
                call(&app, "POST", "/me/mfa/totp/confirm", Some(&admin), Some(body)).await;

            let events = recorded_events(&services.database).await;
            let names: Vec<&str> = events.iter().map(|(name, ..)| name.as_str()).collect();
            assert_eq!(
                names,
                [
                    "auth.user_created", // `ada` của router test
                    "auth.user_created", // root, bằng CLI
                    "auth.signed_in",
                    "auth.user_created",
                    "auth.email_verification_requested", // mail xác minh cho bob
                    "auth.role_created",
                    "auth.user_roles_changed",
                    "auth.user_updated",
                    "auth.api_key_created",
                    "auth.mfa_enabled",
                ]
            );
            // Do người tạo bằng CLI ("hệ thống") và do quản trị viên làm.
            assert_eq!(events[1].1, None);
            assert!(events[3..].iter().all(|(_, actor, ..)| *actor == Some(root.id)));
            assert_eq!(events[3].2.as_deref(), Some(format!("auth.user:{bob_id}").as_str()));
            let disabled: serde_json::Value = serde_json::from_str(&events[7].3).unwrap();
            assert!(disabled["before"]["disabled_at"].is_null());
            assert!(disabled["after"]["disabled_at"].is_string());
            let roles: serde_json::Value = serde_json::from_str(&events[6].3).unwrap();
            assert_eq!(roles, json!({"before": [], "after": ["reader"]}));

            let everything: String = events.iter().map(|(.., payload)| payload.as_str()).collect();
            for secret_value in [
                PASSWORD,
                key["data"]["key"].as_str().unwrap(),
                &secret,
                codes["data"]["recovery_codes"][0].as_str().unwrap(),
                &crate::testing::last_token(&services),
            ] {
                assert!(!everything.contains(secret_value), "a secret leaked into an event");
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn quan_tri_vien_dat_mat_khau_cho_tai_khoan_chua_co() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let body = json!({"email": "bob@example.com", "name": "Bob"});
            let (_, _, created) = call(&app, "POST", "/users", Some(&admin), Some(body)).await;
            let path = format!("/users/{}/password", created["data"]["id"].as_str().unwrap());

            let body = json!({"password": PASSWORD});
            let (status, _, _) = call(&app, "PUT", &path, Some(&admin), Some(body.clone())).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            bearer(&app, "bob@example.com", PASSWORD).await;
            let (status, _, problem) = call(&app, "PUT", &path, Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(problem["code"], "auth.password_already_set");
        })
        .await
        .unwrap();
    }
}
