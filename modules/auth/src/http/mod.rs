//! Endpoint REST của module `auth`, dưới `/api/v1/auth`. Handler chỉ lấy dữ liệu
//! vào, gọi service và trả envelope; mọi logic nằm ở service.

mod account;
pub(crate) mod api_keys;
mod customers;
mod delegations;
mod email;
pub(crate) mod invitations;
mod login_link;
pub(crate) mod memberships;
mod mfa;
pub(crate) mod oidc;
mod passkeys;
mod reauth;
mod reset;
pub(crate) mod roles;
pub(crate) mod session;
mod sessions;
mod support;
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
    /// Đăng nhập qua nhà cung cấp danh tính (bước 40 P3).
    pub oidc: Arc<crate::oidc::OidcService>,
    pub sessions: Arc<SessionService>,
    pub users: Arc<UserService>,
    pub roles: Arc<RoleService>,
    pub api_keys: Arc<ApiKeyService>,
    pub mfa: Arc<MfaService>,
    pub resets: Arc<PasswordResets>,
    pub verifications: Arc<EmailVerifications>,
    pub passkeys: Arc<crate::passkey::PasskeyService>,
    /// Cây tổ chức: cài đặt của khách (`auth.customer_registration`…). `None`: app không có
    /// module `organizations`.
    pub orgs: Option<Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    /// Quyền vào hỗ trợ của nhân sự nền tảng (D-313).
    pub support: Arc<crate::support::SupportService>,
    /// Lời mời người dùng qua email (D-312).
    pub invitations: Arc<crate::invitation::Invitations>,
    /// Đăng nhập bằng link qua email (D-311).
    pub login_links: Arc<crate::login_link::LoginLinks>,
    /// Uỷ quyền theo cây khách hàng (D-310).
    pub delegations: Arc<crate::delegation::DelegationService>,
    /// Xác nhận lại danh tính (`POST /me/reauth`, D-308).
    pub reauth: Arc<crate::reauth::Reauthentication>,
    /// Dữ liệu cá nhân của mọi module, cho `GET /me/export`.
    pub personal_data: autoxide_kernel::privacy::PersonalDataExport,
    /// Cho người lạ tự đăng ký (`POST /register`).
    pub self_registration: bool,
    /// Header mang quốc gia của người gọi do proxy tin cậy đặt (`AUTOXIDE_AUTH_COUNTRY_HEADER`,
    /// D-272); `None`: phiên không có quốc gia.
    pub country_header: Option<axum::http::HeaderName>,
}

pub(crate) fn routes(state: HttpState) -> OpenApiRouter {
    session::routes()
        .merge(account::routes())
        .merge(roles::routes())
        .merge(memberships::routes())
        .merge(users::routes())
        .merge(api_keys::routes())
        .merge(mfa::routes())
        .merge(reset::routes())
        .merge(email::routes())
        .merge(passkeys::routes())
        .merge(passkeys::login_routes())
        .merge(sessions::routes())
        .merge(reauth::routes())
        .merge(customers::routes())
        .merge(delegations::routes())
        .merge(login_link::routes())
        .merge(oidc::routes())
        .merge(invitations::routes())
        .merge(support::routes())
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

/// Ghi quốc gia gần đúng của phiên vừa mở, lấy từ header của proxy tin cậy (D-272). Chỉ nhận mã
/// hai chữ ISO 3166-1 (`XX`, `T1` — "không biết", "Tor" của Cloudflare — bị bỏ). Chỉ là thông tin
/// để người dùng nhận ra phiên: ghi lỗi thì ghi log rồi bỏ qua, không làm hỏng lần đăng nhập.
pub(super) async fn note_country(
    state: &HttpState,
    tenant: &autoxide_kernel::tenancy::TenantContext,
    headers: &axum::http::HeaderMap,
    session_id: uuid::Uuid,
) {
    let Some(name) = &state.country_header else {
        return;
    };
    let Some(country) = headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|code| code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_uppercase()))
        .filter(|code| code != "XX")
    else {
        return;
    };
    if let Err(error) = state.sessions.note_country(tenant, session_id, &country).await {
        tracing::warn!(%session_id, %error, "could not record the country of a session");
    }
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
        let tenant = services.tenant.clone();
        let mailer: std::sync::Arc<dyn Mailer> = services.mails.clone();
        let app = TestApp::builder(test)
            .setting("AUTOXIDE_AUTH_TOKEN_SECRET", crate::testing::TOKEN_SECRET)
            .setting("AUTOXIDE_AUTH_SELF_REGISTRATION", &self_registration.to_string())
            .setting("AUTOXIDE_AUTH_PASSKEY_RP_ID", crate::testing::PASSKEY_RP_ID)
            .setting("AUTOXIDE_AUTH_PASSKEY_ORIGINS", crate::testing::PASSKEY_ORIGIN)
            .setting("AUTOXIDE_AUTH_COUNTRY_HEADER", "CF-IPCountry")
            .provide::<dyn Mailer>(mailer)
            .module(crate::module())
            .module(NotesPermissions)
            .build()
            .await
            .unwrap();
        services
            .users
            .create(
                &tenant,
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

/// Xác nhận chủ tài khoản từ thân request: đúng một trong `current_password` / `reauth_token`
/// (D-308); sai thì 422 cho ô `current_password`.
pub(crate) fn confirm<'a>(
    current_password: &'a str,
    reauth_token: Option<&'a str>,
) -> Result<crate::reauth::Confirm<'a>, autoxide_kernel::error::Error> {
    crate::reauth::Confirm::from_request(current_password, reauth_token)
        .map_err(|problem| autoxide_kernel::error::Error::validation(vec![problem]))
}
