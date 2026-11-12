//! Endpoint REST của module `auth`, dưới `/api/v1/auth`. Handler chỉ lấy dữ liệu
//! vào, gọi service và trả envelope; mọi logic nằm ở service.

mod account;
pub(crate) mod api_keys;
mod email;
mod mfa;
mod passkeys;
mod reset;
pub(crate) mod roles;
pub(crate) mod session;
mod sessions;
pub(crate) mod users;

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::api_key::ApiKeyService;
use crate::mfa::MfaService;
use crate::reset::PasswordResets;
use crate::role::RoleService;
use crate::session::SessionService;
use crate::user::UserService;
use crate::verify::EmailVerifications;

/// Các service mà handler dùng.
#[derive(Clone)]
pub(crate) struct HttpState {
    pub sessions: Arc<SessionService>,
    pub users: Arc<UserService>,
    pub roles: Arc<RoleService>,
    pub api_keys: Arc<ApiKeyService>,
    pub mfa: Arc<MfaService>,
    pub resets: Arc<PasswordResets>,
    pub verifications: Arc<EmailVerifications>,
    pub passkeys: Arc<crate::passkey::PasskeyService>,
    /// Dữ liệu cá nhân của mọi module, cho `GET /me/export`.
    pub personal_data: autoxide_kernel::privacy::PersonalDataExport,
    /// Cho người lạ tự đăng ký (`POST /register`).
    pub self_registration: bool,
}

pub(crate) fn routes(state: HttpState) -> OpenApiRouter {
    session::routes()
        .merge(account::routes())
        .merge(roles::routes())
        .merge(users::routes())
        .merge(api_keys::routes())
        .merge(mfa::routes())
        .merge(reset::routes())
        .merge(email::routes())
        .merge(passkeys::routes())
        .merge(sessions::routes())
        .with_state(Arc::new(state))
}

/// Việc chỉ làm được trong phiên đăng nhập bằng mật khẩu, không bằng API key (vd đổi
/// cài đặt bảo mật của chính tài khoản): key bị lộ không được dùng để chiếm tài khoản.
fn require_session(
    principal: &autoxide_kernel::access::Principal,
) -> Result<(), autoxide_kernel::error::Error> {
    if principal.api_key_id().is_some() {
        return Err(autoxide_kernel::error::Error::forbidden(
            "auth.session_required",
            "This action needs a signed-in session; an API key cannot do it. Sign in with your \
             password (`POST /api/v1/auth/login`) and use that access token.",
        ));
    }
    Ok(())
}

/// Header `User-Agent` của request, để người dùng nhận ra thiết bị trong danh sách phiên.
pub(super) fn user_agent(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers.get(axum::http::header::USER_AGENT).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::access::Principal;
    use uuid::Uuid;

    #[test]
    fn api_key_khong_doi_duoc_cai_dat_bao_mat() {
        let session = Principal::new(Uuid::now_v7(), []);
        let key = session.clone().via_api_key(Uuid::now_v7());

        assert!(super::require_session(&session).is_ok());
        assert_eq!(super::require_session(&key).unwrap_err().code(), "auth.session_required");
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Router đầy đủ (qua mọi lớp chung của kernel) và cách gọi nó trong test.

    use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
    use autoxide_testkit::TestDatabase;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{HeaderMap, Request, StatusCode, header};
    use serde_json::Value;
    use tower::ServiceExt as _;
    use utoipa_axum::router::OpenApiRouter;

    use autoxide_kernel::event::Actor;

    use crate::testing::Services;
    use crate::user::NewUser;

    pub const PASSWORD: &str = "correct horse battery staple";

    /// Router với một người dùng sẵn có, `ada@example.com`; tự đăng ký tắt.
    pub async fn app(test: &TestDatabase) -> (Router, Services) {
        app_with(test, false).await
    }

    /// Như [`app`], chọn bật hay tắt tự đăng ký.
    pub async fn app_with(test: &TestDatabase, self_registration: bool) -> (Router, Services) {
        let services = crate::testing::services(test).await;
        services
            .users
            .create(
                Actor::system(),
                NewUser {
                    email: "ada@example.com".to_owned(),
                    name: "Ada".to_owned(),
                    password: Some(PASSWORD.to_owned()),
                },
            )
            .await
            .unwrap();
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);
        let (config, _) = reader.finish(config).unwrap();
        let state = super::HttpState {
            sessions: services.sessions.clone(),
            users: services.users.clone(),
            roles: services.roles.clone(),
            api_keys: services.api_keys.clone(),
            mfa: services.mfa.clone(),
            resets: services.resets.clone(),
            verifications: services.verifications.clone(),
            passkeys: services.passkeys.clone(),
            personal_data: services.personal_data.clone(),
            self_registration,
        };
        let router = autoxide_kernel::http::build(
            &config.http,
            OpenApiRouter::new(),
            vec![("auth", super::routes(state))],
            autoxide_kernel::http::Extras {
                authenticator: Some(services.authenticator.clone()),
                translations: autoxide_kernel::i18n::Translations::new(vec![(
                    "auth",
                    vec![("vi", include_str!("../../locales/vi.json"))],
                )])
                .unwrap(),
                ..Default::default()
            },
        )
        .router;
        (router, services)
    }

    /// Gọi `method /api/v1/auth{path}`; trả trạng thái, header và thân JSON (`Null`
    /// khi thân rỗng).
    pub async fn call(
        app: &Router,
        method: &str,
        path: &str,
        authorization: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let headers: Vec<_> = authorization
            .map(|value| (header::AUTHORIZATION.as_str(), value))
            .into_iter()
            .collect();
        call_with(app, method, path, &headers, body).await
    }

    /// Như [`call`], với các header tuỳ ý.
    pub async fn call_with(
        app: &Router,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut request = Request::builder().method(method).uri(format!("/api/v1/auth{path}"));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let json =
            if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
        (parts.status, parts.headers, json)
    }

    /// Đăng nhập `email` và trả header `Authorization` dùng được ngay.
    pub async fn bearer(app: &Router, email: &str, password: &str) -> String {
        let body = serde_json::json!({"email": email, "password": password});
        let (status, _, login) = call(app, "POST", "/login", None, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{login}");
        format!("Bearer {}", login["data"]["access_token"].as_str().unwrap())
    }
}
