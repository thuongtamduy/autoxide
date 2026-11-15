//! Endpoint quản lý phiên đăng nhập: người dùng xem và kết thúc phiên của chính mình
//! (từng thiết bị hoặc mọi thiết bị khác); quản trị viên kết thúc mọi phiên của một
//! người.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::HttpState;
use crate::permissions::USERS_MANAGE;
use crate::session::Session;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_sessions, end_sessions))
        .routes(routes!(end_session))
        .routes(routes!(end_user_sessions))
}

/// A signed-in session: one device or client that stays signed in by refreshing its
/// tokens.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct SessionResponse {
    /// Same as `session_id` in the token response of the client that holds it.
    id: Uuid,
    /// The `User-Agent` header sent when signing in (up to 255 characters); `null` when
    /// none was sent.
    #[schema(example = "Mozilla/5.0 (X11; Linux x86_64) Firefox/140.0")]
    user_agent: Option<String>,
    /// How the session was signed in: `password`, `password_and_mfa` (password and a code),
    /// `passkey`, or `password_change` (a new session after changing the password); `null` for
    /// sessions older than this information.
    #[schema(example = "password_and_mfa")]
    sign_in_method: Option<String>,
    /// Approximate country of the sign-in (ISO 3166-1 two-letter code), from the server's
    /// reverse proxy when it is set up for it (`AUTOXIDE_AUTH_COUNTRY_HEADER`); `null` when not
    /// known. The IP address is never kept.
    #[schema(example = "VN")]
    country: Option<String>,
    /// When the session was signed in.
    started_at: DateTime<Utc>,
    /// Last time the session got tokens (sign-in or refresh).
    last_active_at: DateTime<Utc>,
    /// When the session ends unless refreshed before.
    expires_at: DateTime<Utc>,
}

impl From<Session> for SessionResponse {
    fn from(session: Session) -> Self {
        SessionResponse {
            id: session.id,
            user_agent: session.user_agent,
            sign_in_method: session.sign_in_method,
            country: session.country,
            started_at: session.started_at,
            last_active_at: session.last_active_at,
            expires_at: session.expires_at,
        }
    }
}

/// Which sessions to keep when ending several at once.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EndSessionsQuery {
    /// Id of a session to keep signed in, usually your own (`session_id` from the token
    /// response). Omit it to end every session, including this one.
    keep: Option<Uuid>,
}

/// How many sessions were ended.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct EndedSessionsResponse {
    #[schema(example = 3)]
    ended: u64,
}

/// List your signed-in sessions.
///
/// Signed in with a password session only (an API key cannot do this). One entry per
/// device or client; newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/me/sessions",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_sessions",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your sessions.", body = Page<SessionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.session_required`. Sign in with `POST /api/v1/auth/login` and use that access token.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_sessions(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<SessionResponse>, Error> {
    super::require_session(&principal)?;
    let page = PageRequest::try_from(query)?;
    let sessions = state.sessions.list_sessions(principal.user_id(), &page).await?;
    Ok(sessions.map(SessionResponse::from))
}

/// Sign out one of your sessions, e.g. a lost device.
///
/// Signed in with a password session only. Its refresh token stops working at once;
/// an access token already issued to it keeps working until it expires (15 minutes by
/// default).
#[utoipa::path(
    delete,
    path = "/me/sessions/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "end_session",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the session.")),
    responses(
        (status = 204, description = "The session was ended."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.session_required`. Sign in with `POST /api/v1/auth/login` and use that access token.", body = Problem),
        (status = 404, description = "You have no active session with this id; code `auth.session_not_found`.", body = Problem),
    )
)]
async fn end_session(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    super::require_session(&principal)?;
    state.sessions.revoke_session(principal.user_id(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sign out all your sessions, or all but one.
///
/// Signed in with a password session only. Pass `keep` with your own `session_id` to
/// sign out every other device and stay signed in here. Safe to repeat.
#[utoipa::path(
    delete,
    path = "/me/sessions",
    extensions(("x-permissions" = json!([]))),
    operation_id = "end_sessions",
    security(("bearer" = [])),
    params(EndSessionsQuery),
    responses(
        (status = 200, description = "The sessions were ended.", body = Data<EndedSessionsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.session_required`. Sign in with `POST /api/v1/auth/login` and use that access token.", body = Problem),
        (status = 422, description = "`keep` is not a valid id.", body = Problem),
    )
)]
async fn end_sessions(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Query(query): Query<EndSessionsQuery>,
) -> Result<Data<EndedSessionsResponse>, Error> {
    super::require_session(&principal)?;
    let user_id = principal.user_id();
    let ended =
        state.sessions.revoke_sessions(Actor::from(&principal), user_id, query.keep).await?;
    Ok(Data { data: EndedSessionsResponse { ended } })
}

/// Sign a user out everywhere.
///
/// Requires `auth.users.manage`, and every permission the user has. Ends all their
/// sessions; their access tokens keep working until they expire (15 minutes by
/// default). To keep them out for good, disable the account instead.
#[utoipa::path(
    delete,
    path = "/users/{id}/sessions",
    extensions(("x-permissions" = json!(["auth.users.manage"]))),
    operation_id = "end_user_sessions",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    responses(
        (status = 200, description = "The user's sessions were ended.", body = Data<EndedSessionsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.manage` (code `permission_denied`), or the user has permissions you lack (code `auth.target_has_more_permissions`).", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
    )
)]
async fn end_user_sessions(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<EndedSessionsResponse>, Error> {
    principal.require(&USERS_MANAGE)?;
    state.users.get(id).await?.ok_or_else(|| crate::user::user_not_found(id))?;
    state.roles.require_covers(&principal, id).await?;
    let ended = state.sessions.revoke_sessions(Actor::from(&principal), id, None).await?;
    Ok(Data { data: EndedSessionsResponse { ended } })
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::tenancy::TenantContext;
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::{Value, json};

    use crate::api::{Accounts, NewAccount};
    use crate::http::test_support::{PASSWORD, app, bearer, call, call_with};

    /// Đăng nhập `ada` từ một "thiết bị" (User-Agent); trả (Authorization, thân token).
    async fn sign_in(app: &autoxide_testkit::TestApp, device: &str) -> (String, Value) {
        let body = json!({"email": "ada@example.com", "password": PASSWORD});
        let headers = [("user-agent", device)];
        let (status, _, login) = call_with(app, "POST", "/login", &headers, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{login}");
        let authorization = format!("Bearer {}", login["data"]["access_token"].as_str().unwrap());
        (authorization, login["data"].clone())
    }

    #[tokio::test]
    async fn xem_va_dang_xuat_tung_thiet_bi() {
        TestDatabase::run(async |test| {
            let (app, _) = app(test).await;
            let (phone, phone_tokens) = sign_in(&app, "Phone").await;
            let (_, laptop_tokens) = sign_in(&app, "Laptop").await;
            let (_, tablet_tokens) = sign_in(&app, "Tablet").await;

            // Refresh giữ nguyên phiên và thiết bị.
            let body = json!({"refresh_token": laptop_tokens["refresh_token"]});
            let (status, _, refreshed) = call(&app, "POST", "/refresh", None, Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{refreshed}");
            assert_eq!(refreshed["data"]["session_id"], laptop_tokens["session_id"]);

            let (status, _, list) = call(&app, "GET", "/me/sessions", Some(&phone), None).await;
            assert_eq!(status, StatusCode::OK, "{list}");
            let devices: Vec<_> =
                list["data"].as_array().unwrap().iter().map(|s| s["user_agent"].clone()).collect();
            assert_eq!(devices, vec![json!("Tablet"), json!("Laptop"), json!("Phone")]);
            assert_eq!(list["data"][1]["id"], laptop_tokens["session_id"]);

            // Đăng xuất chiếc laptop: refresh token của nó hết dùng được.
            let path = format!("/me/sessions/{}", laptop_tokens["session_id"].as_str().unwrap());
            let (status, _, _) = call(&app, "DELETE", &path, Some(&phone), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let body = json!({"refresh_token": refreshed["data"]["refresh_token"]});
            let (status, _, _) = call(&app, "POST", "/refresh", None, Some(body)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let (status, _, again) = call(&app, "DELETE", &path, Some(&phone), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(again["code"], "auth.session_not_found");

            // Đăng xuất mọi thiết bị khác, giữ chiếc điện thoại.
            let path =
                format!("/me/sessions?keep={}", phone_tokens["session_id"].as_str().unwrap());
            let (status, _, ended) = call(&app, "DELETE", &path, Some(&phone), None).await;
            assert_eq!(status, StatusCode::OK, "{ended}");
            assert_eq!(ended["data"]["ended"], 1);
            let body = json!({"refresh_token": tablet_tokens["refresh_token"]});
            let (status, _, _) = call(&app, "POST", "/refresh", None, Some(body)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let (_, _, list) = call(&app, "GET", "/me/sessions", Some(&phone), None).await;
            assert_eq!(list["data"].as_array().unwrap().len(), 1);
            assert_eq!(list["data"][0]["id"], phone_tokens["session_id"]);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn quan_tri_vien_dang_xuat_moi_phien_cua_nguoi_dung() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(
                services.users.as_ref(),
                &TenantContext::legacy_default(&services.database),
                root,
            )
            .await
            .unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let (ada, ada_tokens) = sign_in(&app, "Phone").await;
            sign_in(&app, "Laptop").await;
            let ada_id = services.users.find_by_email("ada@example.com").await.unwrap().unwrap().id;

            // Người không có quyền thì không được; người dùng thường không đụng được admin.
            let path = format!("/users/{ada_id}/sessions");
            let (status, _, _) = call(&app, "DELETE", &path, Some(&ada), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);

            let (status, _, ended) = call(&app, "DELETE", &path, Some(&admin), None).await;
            assert_eq!(status, StatusCode::OK, "{ended}");
            assert_eq!(ended["data"]["ended"], 2);
            let body = json!({"refresh_token": ada_tokens["refresh_token"]});
            let (status, _, _) = call(&app, "POST", "/refresh", None, Some(body)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            let missing = format!("/users/{}/sessions", uuid::Uuid::now_v7());
            let (status, _, _) = call(&app, "DELETE", &missing, Some(&admin), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND);

            // Audit: một event cho cả lượt.
            let events = crate::testing::recorded_events(&services.database).await;
            let revoked: Vec<_> =
                events.iter().filter(|event| event.0 == "auth.sessions_revoked").collect();
            assert_eq!(revoked.len(), 1);
        })
        .await
        .unwrap();
    }

    /// Đề bài giao diện D: phiên mang cách đăng nhập và quốc gia gần đúng từ header của proxy
    /// (`AUTOXIDE_AUTH_COUNTRY_HEADER`, test đặt `CF-IPCountry`); mã lạ bị bỏ; refresh giữ nguyên.
    #[tokio::test]
    async fn phien_kem_cach_dang_nhap_va_quoc_gia() {
        TestDatabase::run(async |test| {
            let (app, _) = app(test).await;
            let body = json!({"email": "ada@example.com", "password": PASSWORD});
            let headers = [("user-agent", "Phone"), ("cf-ipcountry", "vn")];
            let (status, _, phone) =
                call_with(&app, "POST", "/login", &headers, Some(body.clone())).await;
            assert_eq!(status, StatusCode::OK, "{phone}");
            let headers = [("user-agent", "Tor"), ("cf-ipcountry", "XX")];
            let (_, _, hidden) = call_with(&app, "POST", "/login", &headers, Some(body)).await;
            let refresh = json!({"refresh_token": phone["data"]["refresh_token"]});
            let (status, _, _) = call(&app, "POST", "/refresh", None, Some(refresh)).await;
            assert_eq!(status, StatusCode::OK);

            let auth = format!("Bearer {}", hidden["data"]["access_token"].as_str().unwrap());
            let (status, _, list) = call(&app, "GET", "/me/sessions", Some(&auth), None).await;
            assert_eq!(status, StatusCode::OK, "{list}");
            let sessions = list["data"].as_array().unwrap();
            let find =
                |agent: &str| sessions.iter().find(|s| s["user_agent"] == agent).unwrap().clone();
            assert_eq!(
                (find("Phone")["country"].as_str(), find("Phone")["sign_in_method"].as_str()),
                (Some("VN"), Some("password"))
            );
            assert!(find("Tor")["country"].is_null(), "{list}");
        })
        .await
        .unwrap();
    }
}
