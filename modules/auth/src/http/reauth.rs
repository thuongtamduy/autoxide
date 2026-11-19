//! `POST /me/reauth`: xác nhận lại danh tính, nhận token ngắn hạn cho một lệnh nhạy cảm (D-308).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::api_key::Reauth;
use crate::reauth::Proof;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new().routes(routes!(reauthenticate))
}

/// How you prove it is you.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReauthMethod {
    /// Your current password, in `password`.
    Password,
    /// A code from your authenticator app, in `totp_code`.
    Totp,
    /// One of your passkeys: `passkey_ceremony_id` and `passkey_credential`, from
    /// `POST /api/v1/auth/login/passkey/options`.
    Passkey,
    /// A mail to your address with the token.
    EmailLink,
}

/// Proof that you are the account owner, for one sensitive change.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReauthRequest {
    #[schema(inline)]
    method: ReauthMethod,
    /// With `password`.
    #[validate(length(max = 1024))]
    password: Option<String>,
    /// With `totp`.
    #[schema(example = "123456")]
    #[validate(length(max = 64))]
    totp_code: Option<String>,
    /// With `passkey`.
    passkey_ceremony_id: Option<Uuid>,
    /// With `passkey`: the browser's `AuthenticationResponseJSON`.
    #[schema(value_type = Option<Object>)]
    passkey_credential: Option<serde_json::Value>,
}

/// A confirmation token.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ReauthResponse {
    /// Send it as `reauth_token` with the sensitive change (creating an API key, changing your
    /// email, turning on two-step sign-in, adding a passkey, deleting your account). Used once.
    /// `null` with `email_link`: the token is in the mail.
    reauth_token: Option<String>,
    /// When the token stops working (`AUTOXIDE_AUTH_REAUTH_MINUTES`).
    expires_at: DateTime<Utc>,
    /// A new access token for this session, confirmed just now: use it to retry an action that
    /// answered 401 `step_up_required`. Sensitive actions then work for
    /// `AUTOXIDE_AUTH_STEP_UP_MINUTES` minutes (refreshing keeps the confirmation). `null` with
    /// `email_link`, and for access tokens issued before this version.
    access_token: Option<String>,
}

fn missing(field: &str) -> Error {
    Error::validation(vec![FieldError::new(field, "required", "is required with this method")])
}

/// Confirm it is you, for a sensitive change.
///
/// Any signed-in user, with a session (not an API key). Prove it with your password, an
/// authenticator code, a passkey, or a mail to your address; the answer (or the mail) carries a
/// short-lived token used once instead of `current_password`. Accounts without a password use
/// this to make sensitive changes. Wrong passwords and codes count towards the sign-in lock.
///
/// Also the answer to 401 `step_up_required`: except with `email_link`, the response carries a new
/// `access_token` for this session, confirmed now; retry the sensitive action with it. Use a
/// passkey or an authenticator code when the account has one.
#[utoipa::path(
    post,
    path = "/me/reauth",
    extensions(("x-permissions" = json!([]))),
    operation_id = "reauthenticate",
    security(("bearer" = [])),
    request_body = ReauthRequest,
    responses(
        (status = 200, description = "Confirmed; the token is in `reauth_token`.", body = Data<ReauthResponse>),
        (status = 202, description = "With `email_link`: the token is on its way to your address.", body = Data<ReauthResponse>),
        (status = 400, description = "The password is wrong; code `auth.wrong_password`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Called with an API key; code `auth.session_required`.", body = Problem),
        (status = 422, description = "A field the method needs is missing, or the code or passkey answer is wrong; see `errors`.", body = Problem),
        (status = 429, description = "Too many wrong attempts for this email; code `auth.login_throttled`.", body = Problem),
    )
)]
async fn reauthenticate(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    headers: axum::http::HeaderMap,
    ValidJson(input): ValidJson<ReauthRequest>,
) -> Result<(StatusCode, Data<ReauthResponse>), Error> {
    super::require_session(&principal)?;
    let proof = match input.method {
        ReauthMethod::Password => {
            Proof::Password(input.password.ok_or_else(|| missing("password"))?)
        }
        ReauthMethod::Totp => {
            Proof::Factor(Reauth::Totp(input.totp_code.ok_or_else(|| missing("totp_code"))?))
        }
        ReauthMethod::Passkey => Proof::Factor(Reauth::Passkey {
            ceremony_id: input.passkey_ceremony_id.ok_or_else(|| missing("passkey_ceremony_id"))?,
            credential: input.passkey_credential.unwrap_or_default(),
        }),
        ReauthMethod::EmailLink => Proof::EmailLink,
    };
    let status =
        if input.method == ReauthMethod::EmailLink { StatusCode::ACCEPTED } else { StatusCode::OK };
    let by_mail = input.method == ReauthMethod::EmailLink;
    let issued = state.reauth.issue(&tenant, principal.user_id(), proof, language).await?;
    // Phiên của chính token đang gọi được ghi là vừa xác thực (bước 54 P5).
    let authorization =
        headers.get(axum::http::header::AUTHORIZATION).and_then(|value| value.to_str().ok());
    let session = crate::session::bearer_token(authorization)
        .and_then(|token| state.sessions.verify_access(token))
        .ok()
        .and_then(|verified| verified.session);
    let access_token = match session {
        Some(session) if !by_mail => {
            Some(state.sessions.reconfirm(&tenant, principal.user_id(), session).await?)
        }
        _ => None,
    };
    let data =
        ReauthResponse { reauth_token: issued.token, expires_at: issued.expires_at, access_token };
    Ok((status, Data { data }))
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::event::Actor;
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::http::test_support::{PASSWORD, app, bearer, call};
    use crate::mfa::service::tests::code_at;

    #[tokio::test]
    async fn xac_nhan_lai_roi_lam_lenh_nhay_cam_khong_can_mat_khau() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;

            let wrong = json!({"method": "password", "password": "not it"});
            let (status, _, refused) =
                call(&app, "POST", "/me/reauth", Some(&ada), Some(wrong)).await;
            assert_eq!(refused["code"], "auth.wrong_password", "{status}");
            let missing = json!({"method": "totp"});
            let (status, _, refused) =
                call(&app, "POST", "/me/reauth", Some(&ada), Some(missing)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
            assert_eq!(refused["errors"][0]["field"], "totp_code");

            let body = json!({"method": "password", "password": PASSWORD});
            let (status, _, issued) =
                call(&app, "POST", "/me/reauth", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{issued}");
            let token = issued["data"]["reauth_token"].as_str().unwrap().to_owned();

            // Gửi cả mật khẩu lẫn token: 422, token chưa bị dùng.
            let both = json!({"current_password": PASSWORD, "reauth_token": token});
            let path = "/me/passkeys/registration";
            let (status, _, refused) = call(&app, "POST", path, Some(&ada), Some(both)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
            let body = json!({"reauth_token": token});
            let (status, _, started) =
                call(&app, "POST", path, Some(&ada), Some(body.clone())).await;
            assert_eq!(status, StatusCode::OK, "{started}");
            // Dùng một lần.
            let (status, _, again) = call(&app, "POST", path, Some(&ada), Some(body)).await;
            assert_eq!(
                (status, again["code"].as_str()),
                (StatusCode::FORBIDDEN, Some("auth.reauth_token_invalid"))
            );

            // Qua email: token nằm trong mail, dùng để bật TOTP.
            let body = json!({"method": "email_link"});
            let (status, _, mailed) =
                call(&app, "POST", "/me/reauth", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::ACCEPTED, "{mailed}");
            assert_eq!(mailed["data"]["reauth_token"], serde_json::Value::Null);
            let token = crate::testing::last_token(&services);
            let body = json!({"reauth_token": token});
            let (status, _, setup) =
                call(&app, "POST", "/me/mfa/totp", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{setup}");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn doi_ten_ngon_ngu_mui_gio_cua_minh() {
        TestDatabase::run(async |test| {
            let (app, _) = app(test).await;
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;
            let body =
                json!({"name": " Ada L. ", "locale": "vi-VN", "time_zone": "Asia/Ho_Chi_Minh"});
            let (status, _, me) = call(&app, "PATCH", "/me", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{me}");
            assert_eq!(
                (&me["data"]["name"], &me["data"]["locale"], &me["data"]["time_zone"]),
                (&json!("Ada L."), &json!("vi"), &json!("Asia/Ho_Chi_Minh"))
            );
            let bad = json!({"locale": "fr", "time_zone": "Mars/Base"});
            let (status, _, refused) = call(&app, "PATCH", "/me", Some(&ada), Some(bad)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
            let fields: Vec<&str> = refused["errors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|error| error["field"].as_str().unwrap())
                .collect();
            assert_eq!(fields, ["locale", "time_zone"], "báo mọi lỗi một lần");
            let clear = json!({"locale": null});
            let (_, _, me) = call(&app, "PATCH", "/me", Some(&ada), Some(clear)).await;
            assert_eq!(me["data"]["locale"], serde_json::Value::Null);
            assert_eq!(me["data"]["time_zone"], "Asia/Ho_Chi_Minh", "không gửi: giữ nguyên");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn role_bat_buoc_mfa_cho_toi_khi_bat_mfa() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let ada_id =
                services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap().id;
            let role = services
                .roles
                .create(
                    &tenant,
                    Actor::system(),
                    crate::role::RoleInput {
                        denies: Vec::new(),
                        requires_mfa: true,
                        name: "writer".to_owned(),
                        description: String::new(),
                        permissions: vec!["notes.write".to_owned()],
                        grants: Vec::new(),
                    },
                )
                .await
                .unwrap();
            assert!(role.requires_mfa);
            services
                .roles
                .set_user_roles(&tenant, Actor::system(), ada_id, vec![role.id])
                .await
                .unwrap();
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;
            let (_, _, me) = call(&app, "GET", "/me", Some(&ada), None).await;
            assert_eq!(me["data"]["mfa_required"], true, "{me}");
            assert_eq!(me["data"]["permissions"], json!([]), "quyền của role bị giữ");

            let body = json!({"current_password": PASSWORD});
            let (_, _, setup) = call(&app, "POST", "/me/mfa/totp", Some(&ada), Some(body)).await;
            let secret = setup["data"]["secret"].as_str().unwrap().to_owned();
            let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap();
            let body = json!({"code": code_at(&secret, now)});
            let (status, _, on) =
                call(&app, "POST", "/me/mfa/totp/confirm", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{on}");
            let (_, _, me) = call(&app, "GET", "/me", Some(&ada), None).await;
            assert_eq!(me["data"]["mfa_required"], false);
            assert_eq!(me["data"]["permissions"], json!(["notes.write"]));

            // Mã sai khi xác nhận lại: lỗi chỉ đúng ô `totp_code` client đã gửi, không phải `code`.
            let stale = code_at(&secret, now - 3600);
            let body = json!({"method": "totp", "totp_code": stale});
            let (status, _, refused) =
                call(&app, "POST", "/me/reauth", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
            assert_eq!(refused["errors"][0]["field"], "totp_code", "{refused}");
        })
        .await
        .unwrap();
    }
}
