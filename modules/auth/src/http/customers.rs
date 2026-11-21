//! Tài khoản người mua của khách (D-309): xem, đổi tên, khoá / mở. Không có role, không permission;
//! nhân sự quản lý qua `/users`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::{Principal, Realm};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use super::users::UserStatusFilter;
use crate::api::User;
use crate::permissions::{CUSTOMERS_MANAGE, CUSTOMERS_READ};
use crate::user::{UserChanges, UserStatus};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_customers))
        .routes(routes!(get_customer, update_customer))
}

/// A customer account of the tenant's shop.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct CustomerResponse {
    id: Uuid,
    #[schema(example = "buyer@example.com")]
    email: String,
    #[schema(example = "Nguyen Van A")]
    name: String,
    /// When the account was disabled; `null` while active.
    disabled_at: Option<Timestamp>,
    /// When the email was confirmed; `null` while not verified.
    email_verified_at: Option<Timestamp>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<User> for CustomerResponse {
    fn from(user: User) -> Self {
        CustomerResponse {
            id: user.id,
            email: user.email,
            name: user.name,
            disabled_at: user.disabled_at,
            email_verified_at: user.email_verified_at,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

/// Filters for the customer list.
#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct CustomerQuery {
    /// Only customers whose email or name contains this text, without regard to case or accents.
    #[validate(length(max = 200))]
    q: Option<String>,
    /// Only `active` or only `disabled` accounts.
    #[param(inline)]
    status: Option<UserStatusFilter>,
}

/// Changes to a customer account. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateCustomerRequest {
    /// New display name, 1 to 200 characters.
    name: Option<String>,
    /// `true` disables the account and signs it out everywhere; `false` enables it.
    disabled: Option<bool>,
}

/// List or search the customer accounts of the shop.
///
/// Requires `auth.customers.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/customers",
    extensions(("x-permissions" = json!(["auth.customers.read"]))),
    operation_id = "list_customers",
    security(("bearer" = [])),
    params(CustomerQuery, PageQuery),
    responses(
        (status = 200, description = "One page of customer accounts.", body = Page<CustomerResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.customers.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_customers(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<CustomerQuery>,
    Query(query): Query<PageQuery>,
) -> Result<Page<CustomerResponse>, Error> {
    principal.require(&CUSTOMERS_READ)?;
    filter.validate().map_err(Error::from)?;
    let page = PageRequest::try_from(query)?;
    let status = filter.status.map(UserStatus::from);
    let found =
        state.users.list_in(&tenant, Realm::Customer, filter.q.as_deref(), status, &page).await?;
    Ok(found.map(CustomerResponse::from))
}

async fn customer(state: &HttpState, tenant: &TenantContext, id: Uuid) -> Result<User, Error> {
    state
        .users
        .get(tenant, id)
        .await?
        .filter(|user| user.realm == Realm::Customer)
        .ok_or_else(|| customer_not_found(id))
}

/// Get a customer account.
///
/// Requires `auth.customers.read`.
#[utoipa::path(
    get,
    path = "/customers/{id}",
    extensions(("x-permissions" = json!(["auth.customers.read"]))),
    operation_id = "get_customer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer account.")),
    responses(
        (status = 200, description = "The account.", body = Data<CustomerResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.customers.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No customer account has this id; code `auth.customer_not_found`.", body = Problem),
    )
)]
async fn get_customer(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<CustomerResponse>, Error> {
    principal.require(&CUSTOMERS_READ)?;
    let user = customer(&state, &tenant, id).await?;
    Ok(Tagged { version: user.updated_at, data: user.into() })
}

/// Rename, disable or enable a customer account.
///
/// Requires `auth.customers.manage`. Disabling signs the customer out everywhere.
#[utoipa::path(
    patch,
    path = "/customers/{id}",
    extensions(("x-permissions" = json!(["auth.customers.manage"]))),
    operation_id = "update_customer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the customer account.")),
    request_body = UpdateCustomerRequest,
    responses(
        (status = 200, description = "The account after the change.", body = Data<CustomerResponse>, headers(("ETag" = String, description = "New version of the record."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.customers.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No customer account has this id; code `auth.customer_not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_customer(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateCustomerRequest>,
) -> Result<Tagged<CustomerResponse>, Error> {
    principal.require(&CUSTOMERS_MANAGE)?;
    customer(&state, &tenant, id).await?;
    let changes = UserChanges { name: input.name, disabled: input.disabled, ..Default::default() };
    let user = state
        .users
        .update(&tenant, Actor::from(&principal), id, changes, if_match.expected()?)
        .await?;
    Ok(Tagged { version: user.updated_at, data: user.into() })
}

fn customer_not_found(id: Uuid) -> Error {
    Error::not_found("auth.customer_not_found", "There is no customer account `{id}`.")
        .with_param("id", id)
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::access::Realm;
    use autoxide_kernel::event::Actor;
    use autoxide_kernel::tenancy::{MAIN_PLACEMENT, TenantContext, TenantId, TenantStatus};
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;
    use uuid::Uuid;

    use crate::http::test_support::{PASSWORD, app, bearer, call};
    use crate::user::NewUser;

    fn buyer(email: &str) -> NewUser {
        NewUser {
            email: email.to_owned(),
            name: "Buyer".to_owned(),
            password: Some(PASSWORD.to_owned()),
        }
    }

    #[tokio::test]
    async fn nguoi_mua_hai_khach_cung_email_la_hai_tai_khoan() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let a = services.tenant.clone();
            let b = TenantContext::new(
                TenantId::from_uuid(Uuid::now_v7()),
                "shop-b",
                "Shop B",
                TenantStatus::Active,
                MAIN_PLACEMENT,
                services.database.clone(),
            );
            let users = &services.users;
            let in_a =
                users.create_customer(&a, Actor::system(), buyer("ann@example.com")).await.unwrap();
            let in_b =
                users.create_customer(&b, Actor::system(), buyer("ANN@example.com")).await.unwrap();
            assert_ne!(in_a.id, in_b.id);
            // Cùng khách: nhân sự và người mua cùng email là hai tài khoản; người mua thứ hai thì không.
            let staff = users.create(&a, Actor::system(), buyer("ann@example.com")).await.unwrap();
            assert_eq!((staff.realm, in_a.realm), (Realm::Staff, Realm::Customer));
            let again = users.create_customer(&a, Actor::system(), buyer("ann@example.com")).await;
            assert_eq!(again.unwrap_err().code(), "auth.email_taken");
            let found =
                users.find_by_email_in(&b, Realm::Customer, "ann@example.com").await.unwrap();
            assert_eq!(found.map(|user| user.id), Some(in_b.id));
            assert_eq!(
                users.find_by_email(&b, "ann@example.com").await.unwrap(),
                None,
                "B không có nhân sự này"
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn nguoi_mua_khong_co_permission_nao() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            // Đăng ký người mua mở mặc định, kể cả khi máy chủ tắt tự đăng ký của nhân sự.
            let body = json!({"email": "ann@example.com", "name": "Ann", "password": PASSWORD, "realm": "customer"});
            let (status, _, created) = call(&app, "POST", "/register", None, Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            let staff = json!({"email": "bob@example.com", "name": "Bob", "password": PASSWORD});
            let (status, _, _) = call(&app, "POST", "/register", None, Some(staff)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "nhân sự vẫn theo cấu hình máy chủ");

            // Đăng nhập nhầm realm: như sai mật khẩu.
            let wrong = json!({"email": "ann@example.com", "password": PASSWORD});
            let (status, _, _) = call(&app, "POST", "/login", None, Some(wrong)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let login = json!({"email": "ann@example.com", "password": PASSWORD, "realm": "customer"});
            let (status, _, tokens) = call(&app, "POST", "/login", None, Some(login)).await;
            assert_eq!(status, StatusCode::OK, "{tokens}");
            let ann = format!("Bearer {}", tokens["data"]["access_token"].as_str().unwrap());
            let (_, _, me) = call(&app, "GET", "/me", Some(&ann), None).await;
            assert_eq!(me["data"]["permissions"], json!([]));
            let (status, _, _) = call(&app, "GET", "/users", Some(&ann), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);

            // Không gán role được cho người mua; nhân sự quản lý người mua qua `/customers`.
            let ann_id = created["data"]["id"].as_str().unwrap().to_owned();
            let admin_id = services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap().id;
            let write = tenant.database().begin_write().await.unwrap();
            crate::role::grant_admin(&write, &tenant, admin_id).await.unwrap();
            write.commit().await.unwrap();
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;
            let roles = json!({"role_ids": []});
            let (status, _, refused) =
                call(&app, "PUT", &format!("/users/{ann_id}/roles"), Some(&ada), Some(roles)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
            let (status, _, _) = call(&app, "GET", &format!("/users/{ann_id}"), Some(&ada), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            let (_, _, listed) = call(&app, "GET", "/customers?q=ann", Some(&ada), None).await;
            assert_eq!(listed["data"][0]["id"], ann_id.as_str(), "{listed}");
            let (_, _, users) = call(&app, "GET", "/users", Some(&ada), None).await;
            assert!(users["data"].as_array().unwrap().iter().all(|user| user["id"] != ann_id.as_str()));
            let path = format!("/customers/{ann_id}");
            let (status, _, disabled) =
                call(&app, "PATCH", &path, Some(&ada), Some(json!({"disabled": true}))).await;
            assert_eq!(status, StatusCode::OK, "{disabled}");
            let (status, _, _) = call(&app, "GET", "/me", Some(&ann), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "khoá: đăng xuất mọi nơi");
            let staff_path = format!("/customers/{admin_id}");
            let (status, _, _) = call(&app, "GET", &staff_path, Some(&ada), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "nhân sự không phải người mua");
        })
        .await
        .unwrap();
    }
}
