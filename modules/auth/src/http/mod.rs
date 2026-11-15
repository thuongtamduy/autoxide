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
        // Response của `auth` hay chứa secret chỉ trả một lần (token, API key mới, secret TOTP,
        // mã khôi phục): không ai được lưu lại — proxy, trình duyệt, lớp idempotency (RFC 6749
        // §5.1, D-149).
        .layer(axum::middleware::map_response(no_store))
}

async fn no_store(mut response: axum::response::Response) -> axum::response::Response {
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

/// Việc chỉ làm được trong phiên đăng nhập bằng mật khẩu, không bằng API key (vd đổi
/// cài đặt bảo mật của chính tài khoản): key bị lộ không được dùng để chiếm tài khoản.
fn require_session(
    principal: &autoxide_kernel::access::Principal,
) -> Result<(), autoxide_kernel::error::Error> {
    if principal.api_key_id().is_some() {
        return Err(autoxide_kernel::error::Error::forbidden(
            "auth.session_required",
            "This action needs you to be signed in with your password; an API key cannot do it.",
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
    //! App đầy đủ (module `auth` thật qua mọi lớp chung của kernel, dựng bằng `TestApp`) và
    //! cách gọi nó trong test.

    use autoxide_kernel::access::Permission;
    use autoxide_kernel::event::Actor;
    use autoxide_kernel::mail::Mailer;
    use autoxide_kernel::module::Module;
    use autoxide_testkit::{TestApp, TestDatabase};
    use axum::http::{Method, StatusCode, header};
    use serde_json::Value;

    use crate::testing::Services;
    use crate::user::NewUser;

    pub const PASSWORD: &str = "correct horse battery staple";

    /// Module `notes` giả: chỉ khai `notes.write`, để role gán được permission của module khác
    /// (như danh mục permission của `crate::testing::services`).
    struct NotesPermissions;

    impl Module for NotesPermissions {
        fn name(&self) -> &'static str {
            "notes"
        }

        fn permissions(&self) -> Vec<Permission> {
            vec![Permission::new("notes.write", "Write notes.")]
        }
    }

    /// App với một người dùng sẵn có, `ada@example.com`; tự đăng ký tắt.
    pub async fn app(test: &TestDatabase) -> (TestApp, Services) {
        app_with(test, false).await
    }

    /// Như [`app`], chọn bật hay tắt tự đăng ký.
    ///
    /// `Services` là service của module dựng riêng trên cùng database, cùng khoá ký token và
    /// cùng cấu hình với module trong app, và dùng chung bên gửi mail giả: thứ một bên ghi,
    /// bên kia đọc được.
    pub async fn app_with(test: &TestDatabase, self_registration: bool) -> (TestApp, Services) {
        let services = crate::testing::services(test).await;
        let mailer: std::sync::Arc<dyn Mailer> = services.mails.clone();
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_AUTH_TOKEN_SECRET", crate::testing::TOKEN_SECRET)
            .setting("AUTOXIDE_AUTH_SELF_REGISTRATION", &self_registration.to_string())
            .setting("AUTOXIDE_AUTH_PASSKEY_RP_ID", crate::testing::PASSKEY_RP_ID)
            .setting("AUTOXIDE_AUTH_PASSKEY_ORIGINS", crate::testing::PASSKEY_ORIGIN)
            .provide::<dyn Mailer>(mailer)
            .module(crate::module())
            .module(NotesPermissions)
            .build()
            .await
            .unwrap();
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
        (app, services)
    }

    /// Gọi `method /api/v1/auth{path}`; trả trạng thái, header và thân JSON (`Null`
    /// khi thân rỗng).
    pub async fn call(
        app: &TestApp,
        method: &str,
        path: &str,
        authorization: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let headers: Vec<_> = authorization
            .map(|value| (header::AUTHORIZATION.as_str(), value))
            .into_iter()
            .collect();
        call_with(app, method, path, &headers, body).await
    }

    /// Như [`call`], với các header tuỳ ý.
    pub async fn call_with(
        app: &TestApp,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> (StatusCode, axum::http::HeaderMap, Value) {
        let method = Method::from_bytes(method.as_bytes()).unwrap();
        let mut request = app.request(method, &format!("/api/v1/auth{path}"));
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let answer = request.send().await.unwrap();
        let json = if answer.body().is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(answer.body()).unwrap()
        };
        (answer.status(), answer.headers().clone(), json)
    }

    /// Đăng nhập `email` và trả header `Authorization` dùng được ngay.
    pub async fn bearer(app: &TestApp, email: &str, password: &str) -> String {
        let body = serde_json::json!({"email": email, "password": password});
        let (status, _, login) = call(app, "POST", "/login", None, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{login}");
        format!("Bearer {}", login["data"]["access_token"].as_str().unwrap())
    }
}
