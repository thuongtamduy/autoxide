//! Endpoint vòng đời tài khoản của chính người dùng: xuất dữ liệu cá nhân, tự xoá tài
//! khoản. Quản trị viên xoá tài khoản người khác ở `users.rs`.

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use super::HttpState;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new().routes(routes!(export_personal_data)).routes(routes!(delete_own_account))
}

/// Confirms deleting your own account.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct DeleteAccountRequest {
    /// Your current password.
    #[validate(length(min = 1, max = 1024))]
    current_password: String,
}

/// Everything the server keeps about you: one entry per module that keeps personal
/// data, named after the module.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(example = json!({"auth": {"profile": {"email": "ada@example.com"}}, "files": []}))]
pub(crate) struct PersonalDataResponse {
    /// Your profile, roles, sessions, API keys, passkeys and two-step sign-in status.
    #[schema(value_type = Object)]
    auth: Value,
    /// The parts of the other modules, by module name.
    #[serde(flatten)]
    #[schema(value_type = Object)]
    modules: BTreeMap<String, Value>,
}

/// Download everything the server keeps about you.
///
/// Signed in with a password session only (an API key cannot do this). The answer has
/// one entry per module that keeps personal data, named after the module (`auth` holds
/// your profile, roles, sessions, API keys, passkeys and two-step sign-in status).
/// Secrets — password, tokens, keys — are never included.
#[utoipa::path(
    get,
    path = "/me/export",
    extensions(("x-permissions" = json!([]))),
    operation_id = "export_personal_data",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Your data, by module.", body = Data<PersonalDataResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.session_required`. Sign in with `POST /api/v1/auth/login` and use that access token.", body = Problem),
    )
)]
async fn export_personal_data(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<PersonalDataResponse>, Error> {
    super::require_session(&principal)?;
    let mut modules: BTreeMap<String, Value> = state
        .personal_data
        .export(&tenant, principal.user_id())
        .await?
        .into_iter()
        .map(|(module, value)| (module.to_owned(), value))
        .collect();
    let auth = modules.remove("auth").unwrap_or(Value::Null);
    Ok(Data { data: PersonalDataResponse { auth, modules } })
}

/// Delete your own account and everything it owns.
///
/// Signed in with a password session only, and needs your current password (an account
/// without a password — `has_password: false` in `GET /api/v1/auth/me` — sets one first with
/// `POST /api/v1/auth/password-reset`: the mail to its own address sets a first password too). The account, its sessions, keys and passkeys
/// go at once; data kept by other modules (notes, files…) is deleted shortly after.
/// This cannot be undone: download your data first with `GET /api/v1/auth/me/export`.
#[utoipa::path(
    delete,
    path = "/me",
    extensions(("x-permissions" = json!([]))),
    operation_id = "delete_own_account",
    security(("bearer" = [])),
    request_body = DeleteAccountRequest,
    responses(
        (status = 204, description = "The account was deleted."),
        (status = 400, description = "The current password is wrong; code `auth.wrong_password`. A user who forgot it resets it with `POST /api/v1/auth/password-reset`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.session_required`. Sign in with `POST /api/v1/auth/login` and use that access token.", body = Problem),
        (status = 409, description = "You are the last active administrator; give the `admin` role to someone else first. Code `auth.last_admin`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
        (status = 429, description = "Too many wrong passwords for this email; code `auth.login_throttled`. Wait the number of seconds in the `Retry-After` header.", body = Problem),
    )
)]
async fn delete_own_account(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    ValidJson(input): ValidJson<DeleteAccountRequest>,
) -> Result<StatusCode, Error> {
    super::require_session(&principal)?;
    state.sessions.delete_own_account(principal.user_id(), &input.current_password).await?;
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
    async fn xuat_du_lieu_roi_tu_xoa_tai_khoan() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;

            let (status, _, export) = call(&app, "GET", "/me/export", Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK, "{export}");
            let auth = &export["data"]["auth"];
            assert_eq!(auth["profile"]["email"], "ada@example.com");
            assert_eq!(auth["sessions"].as_array().unwrap().len(), 1);
            assert!(!export.to_string().contains("password_hash"));

            let wrong = json!({"current_password": "not it"});
            let (status, _, problem) = call(&app, "DELETE", "/me", Some(&ada), Some(wrong)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(problem["code"], "auth.wrong_password");

            let body = json!({"current_password": PASSWORD});
            let (status, _, _) = call(&app, "DELETE", "/me", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            assert!(services.users.find_by_email("ada@example.com").await.unwrap().is_none());
            // Access token còn hạn nhưng tài khoản không còn: bị từ chối.
            let (status, _, _) = call(&app, "GET", "/me", Some(&ada), None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            // Event cho module khác dọn dữ liệu, chỉ mang id.
            let events = crate::testing::recorded_events(&services.database).await;
            let deleted = events.iter().find(|event| event.0 == "auth.user_deleted").unwrap();
            assert!(!deleted.3.contains("ada@example.com"));
            // Email dùng lại được.
            let again = NewAccount {
                email: "ada@example.com".to_owned(),
                name: "Ada".to_owned(),
                password: None,
                admin: false,
            };
            Accounts::create(
                services.users.as_ref(),
                &TenantContext::legacy_default(&services.database),
                again,
            )
            .await
            .unwrap();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn quan_tri_vien_xoa_tai_khoan_nhung_khong_xoa_admin_cuoi() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            let root = Accounts::create(
                services.users.as_ref(),
                &TenantContext::legacy_default(&services.database),
                root,
            )
            .await
            .unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let ada = services.users.find_by_email("ada@example.com").await.unwrap().unwrap();

            let path = format!("/users/{}", root.id);
            let (status, _, problem) = call(&app, "DELETE", &path, Some(&admin), None).await;
            assert_eq!(status, StatusCode::CONFLICT, "{problem}");
            assert_eq!(problem["code"], "auth.last_admin");
            let body = json!({"current_password": PASSWORD});
            let (status, _, _) = call(&app, "DELETE", "/me", Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::CONFLICT);

            let path = format!("/users/{}", ada.id);
            let (status, _, _) = call(&app, "DELETE", &path, Some(&admin), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let (status, _, problem) = call(&app, "DELETE", &path, Some(&admin), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(problem["code"], "auth.user_not_found");
        })
        .await
        .unwrap();
    }
}

/// Đề bài giao diện B1 (D-265): danh sách người dùng kèm dữ kiện tài khoản và role trong một lần
/// gọi; lọc theo trạng thái; `/me` cho biết có mật khẩu chưa và email đang chờ xác minh, huỷ
/// được yêu cầu đổi email.
#[cfg(test)]
mod web_requests {
    use autoxide_kernel::event::Actor;
    use autoxide_kernel::tenancy::TenantContext;
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::api::{Accounts, NewAccount};
    use crate::http::test_support::{PASSWORD, app, bearer, call};

    #[tokio::test]
    async fn danh_sach_nguoi_dung_kem_du_kien_va_role() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = TenantContext::legacy_default(&services.database);
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(services.users.as_ref(), &tenant, root).await.unwrap();
            let passkey_only = NewAccount {
                email: "pk@example.com".to_owned(),
                name: "Passkey".to_owned(),
                password: None,
                admin: false,
            };
            let pk =
                Accounts::create(services.users.as_ref(), &tenant, passkey_only).await.unwrap();
            let changes = crate::user::UserChanges { name: None, disabled: Some(true) };
            services.users.update(Actor::system(), pk.id, changes, None).await.unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;

            let (status, _, page) =
                call(&app, "GET", "/users?include=roles&order=asc", Some(&admin), None).await;
            assert_eq!(status, StatusCode::OK, "{page}");
            let rows = page["data"].as_array().unwrap();
            let by_email = |email: &str| rows.iter().find(|row| row["email"] == email).unwrap();
            let root = by_email("root@example.com");
            assert_eq!(root["roles"][0]["name"], "admin");
            assert_eq!(
                (root["has_password"].as_bool(), root["mfa_enabled"].as_bool()),
                (Some(true), Some(false))
            );
            assert!(root["last_sign_in_at"].is_string(), "signed in above: {root}");
            let pk_row = by_email("pk@example.com");
            assert_eq!(pk_row["has_password"], false);
            assert_eq!(pk_row["roles"], json!([]));
            assert!(pk_row["last_sign_in_at"].is_null());

            // Không `include=roles`: không có trường `roles`.
            let (_, _, plain) = call(&app, "GET", "/users", Some(&admin), None).await;
            assert!(plain["data"][0].get("roles").is_none());
            let (_, _, disabled) =
                call(&app, "GET", "/users?status=disabled", Some(&admin), None).await;
            let emails: Vec<_> =
                disabled["data"].as_array().unwrap().iter().map(|r| r["email"].clone()).collect();
            assert_eq!(emails, [json!("pk@example.com")]);
            let (_, _, active) =
                call(&app, "GET", "/users?status=active", Some(&admin), None).await;
            assert_eq!(active["data"].as_array().unwrap().len(), 2);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn me_co_mat_khau_va_email_dang_cho_va_huy_doi_email() {
        TestDatabase::run(async |test| {
            let (app, _services) = app(test).await;
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;
            let (_, _, me) = call(&app, "GET", "/me", Some(&ada), None).await;
            assert_eq!(me["data"]["has_password"], true);
            assert!(me["data"]["pending_email"].is_null());
            let (status, _, problem) = call(&app, "DELETE", "/me/email", Some(&ada), None).await;
            assert_eq!(
                (status, problem["code"].as_str()),
                (StatusCode::NOT_FOUND, Some("auth.no_pending_email_change"))
            );

            let change = json!({"current_password": PASSWORD, "new_email": "ada@new.example"});
            let (status, _, _) = call(&app, "POST", "/me/email", Some(&ada), Some(change)).await;
            assert_eq!(status, StatusCode::ACCEPTED);
            let (_, _, me) = call(&app, "GET", "/me", Some(&ada), None).await;
            assert_eq!(me["data"]["pending_email"]["email"], "ada@new.example");
            assert!(me["data"]["pending_email"]["expires_at"].is_string());

            let (status, _, _) = call(&app, "DELETE", "/me/email", Some(&ada), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let (_, _, me) = call(&app, "GET", "/me", Some(&ada), None).await;
            assert!(me["data"]["pending_email"].is_null());
            assert_eq!(me["data"]["email"], "ada@example.com");
        })
        .await
        .unwrap();
    }
}
