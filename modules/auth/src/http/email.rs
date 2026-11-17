//! Endpoint xác minh email và đổi email.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{OptionalTenant, TenantContext};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use super::users::UserResponse;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(resend_verification))
        .routes(routes!(resend_user_verification))
        .routes(routes!(change_email, cancel_email_change))
        .merge(
            // Link trong thư vẫn dùng được khi khách chỉ đọc (A-3.11).
            OpenApiRouter::new()
                .routes(routes!(confirm_email))
                .routes(routes!(report_email_change))
                .layer(autoxide_kernel::tenancy::allowed_while_read_only()),
        )
}

/// A new email address for the signed-in account.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct ChangeEmailRequest {
    /// The new address. Case and surrounding spaces do not matter.
    #[schema(example = "ada@example.org")]
    new_email: String,
    /// Your current password.
    #[serde(default)]
    current_password: String,
    /// A confirmation token from `POST /api/v1/auth/me/reauth`, instead of `current_password`
    /// (accounts without a password use this). Exactly one of the two.
    reauth_token: Option<String>,
}

/// The token from a verification mail.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct VerificationConfirmation {
    #[validate(length(min = 1, max = 256))]
    token: String,
}

/// The token from the mail sent to the old address after an email change.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct EmailChangeReport {
    #[validate(length(min = 1, max = 256))]
    token: String,
}

/// Send the verification mail for your email address again.
///
/// The mail (in the language of `Accept-Language`) carries a token valid 24 hours;
/// confirm it with `POST /api/v1/auth/email-verification/confirm`. At most one mail
/// per minute per address.
#[utoipa::path(
    post,
    path = "/me/email/verification",
    extensions(("x-permissions" = json!([]))),
    operation_id = "resend_email_verification",
    security(("bearer" = [])),
    responses(
        (status = 202, description = "A verification mail is on its way."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 409, description = "The address is already verified; code `auth.email_already_verified`.", body = Problem),
    )
)]
async fn resend_verification(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
) -> Result<StatusCode, Error> {
    state.verifications.send_current(&tenant, principal.user_id(), language).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Send the verification mail of a user again.
///
/// Requires `auth.users.manage`, and every permission the user has (you cannot act on someone
/// with more power than you). For a teammate who lost or never got the mail: it goes to the
/// user's current address (in the language of `Accept-Language`), with a token valid 24 hours.
/// At most one mail per minute per address.
#[utoipa::path(
    post,
    path = "/users/{id}/email/verification",
    extensions(("x-permissions" = json!(["auth.users.manage"]))),
    operation_id = "resend_user_email_verification",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    responses(
        (status = 202, description = "A verification mail is on its way to the user."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.manage` (code `permission_denied`), or the user has permissions you lack (code `auth.target_has_more_permissions`).", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 409, description = "The address is already verified; code `auth.email_already_verified`.", body = Problem),
    )
)]
async fn resend_user_verification(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&crate::permissions::USERS_MANAGE)?;
    state.users.get(&tenant, id).await?.ok_or_else(|| crate::user::user_not_found(id))?;
    state.roles.require_covers(&tenant, &principal, id).await?;
    state.verifications.send_current_by(&tenant, Actor::from(&principal), id, language).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Change your email address.
///
/// Needs the current password and a session signed in with a password (an API key
/// cannot do this). A verification mail goes to the NEW address; the email changes
/// only when its token is confirmed with `POST /api/v1/auth/email-verification/confirm`,
/// within 24 hours. Until then the old address stays in use. Wrong passwords count
/// towards the sign-in lock of the email. Once the change is confirmed, the OLD address gets
/// a mail saying so, with a token to report the change if the owner did not make it
/// (`POST /api/v1/auth/email-change/report`, valid 7 days).
#[utoipa::path(
    post,
    path = "/me/email",
    extensions(("x-permissions" = json!([]))),
    operation_id = "change_email",
    security(("bearer" = [])),
    request_body = ChangeEmailRequest,
    responses(
        (status = 202, description = "A verification mail is on its way to the new address."),
        (status = 400, description = "The current password is wrong; code `auth.wrong_password`. A user who forgot it resets it with `POST /api/v1/auth/password-reset`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Called with an API key (code `auth.session_required`), or the account is disabled (code `auth.user_disabled`).", body = Problem),
        (status = 409, description = "Another account has this email (code `auth.email_taken`), or it is already your verified address (code `auth.email_already_verified`).", body = Problem),
        (status = 422, description = "The body is missing a field or the address is invalid; see `errors`.", body = Problem),
        (status = 429, description = "Too many wrong passwords for this email; code `auth.login_throttled`. Wait the number of seconds in the `Retry-After` header.", body = Problem),
    )
)]
async fn change_email(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    ValidJson(input): ValidJson<ChangeEmailRequest>,
) -> Result<StatusCode, Error> {
    super::require_session(&principal)?;
    state
        .verifications
        .change_email_confirmed(
            &tenant,
            principal.user_id(),
            &input.current_password,
            input.reauth_token.as_deref(),
            &input.new_email,
            language,
        )
        .await?;
    Ok(StatusCode::ACCEPTED)
}

/// Cancel a pending email change.
///
/// Any signed-in user, for their own account. The link sent to the new address stops working
/// (as does any other verification link of the account; ask for a new one if needed) and the
/// email stays as it is. See the pending change in `pending_email` of `GET /api/v1/auth/me`.
#[utoipa::path(
    delete,
    path = "/me/email",
    extensions(("x-permissions" = json!([]))),
    operation_id = "cancel_email_change",
    security(("bearer" = [])),
    responses(
        (status = 204, description = "The change was cancelled."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No change is waiting; code `auth.no_pending_email_change`.", body = Problem),
    )
)]
async fn cancel_email_change(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<StatusCode, Error> {
    state.verifications.cancel_change(&tenant, principal.user_id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Confirm an email address with the token from a verification mail.
///
/// Needs no sign-in: the token proves the mail was received. When the token was sent
/// for a new address (`POST /api/v1/auth/me/email`), the account's email becomes that
/// address; sign in with it from now on. Every other verification token of the
/// account stops working. Needs no tenant header: the token names its tenant.
#[utoipa::path(
    post,
    path = "/email-verification/confirm",
    extensions(("x-public" = json!(true))),
    operation_id = "confirm_email_verification",
    request_body = VerificationConfirmation,
    responses(
        (status = 200, description = "Verified; the account as it is now.", body = Data<UserResponse>),
        (status = 400, description = "The token is unknown, expired or used; code `auth.invalid_verification_token`, with `reason` (`expired`, `used` or `unknown`). Ask for a new mail with `POST /api/v1/auth/me/email/verification`.", body = Problem),
        (status = 409, description = "Another account took this address meanwhile; code `auth.email_taken`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
    )
)]
async fn confirm_email(
    State(state): State<Arc<HttpState>>,
    OptionalTenant(tenant): OptionalTenant,
    language: Language,
    ValidJson(input): ValidJson<VerificationConfirmation>,
) -> Result<Data<UserResponse>, Error> {
    // Khách lấy từ token trong link: chạy cả trên tên miền chính không header.
    let (tenant, user) =
        state.verifications.confirm(tenant.as_ref(), &input.token, language).await?;
    Ok(Data { data: super::users::describe(&state.users, &tenant, user).await? })
}

/// Report an email change you did not make.
///
/// Needs no sign-in: the token comes from the mail sent to the old address after the
/// account's email changed. The account is then locked against whoever changed it: signed
/// out everywhere, its password removed (sign-in with a password stops working), its API
/// keys, passkeys, two-step sign-in and pending tokens removed. When no other account took
/// the old address meanwhile, the email goes back to it and a mail to choose a new password
/// is sent there (in the language of `Accept-Language`); otherwise ask an administrator.
/// Needs no tenant header: the token names its tenant.
#[utoipa::path(
    post,
    path = "/email-change/report",
    extensions(("x-public" = json!(true))),
    operation_id = "report_email_change",
    request_body = EmailChangeReport,
    responses(
        (status = 204, description = "Reported; the account is locked and, when the old address was free, given back to it."),
        (status = 400, description = "The token is unknown, expired or used; code `auth.invalid_email_change_report_token`, with `reason` (`expired`, `used` or `unknown`).", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
    )
)]
async fn report_email_change(
    State(state): State<Arc<HttpState>>,
    OptionalTenant(tenant): OptionalTenant,
    language: Language,
    ValidJson(input): ValidJson<EmailChangeReport>,
) -> Result<StatusCode, Error> {
    // Khách lấy từ token trong link: chạy cả trên tên miền chính không header.
    state.verifications.report(tenant.as_ref(), &input.token, language).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::http::test_support::{PASSWORD, app_with, bearer, call};

    /// Token trong mail cuối cùng.
    fn token(services: &crate::testing::Services) -> String {
        let sent = services.mails.sent.lock().unwrap();
        let text = &sent.last().unwrap().text;
        text.split_whitespace().find(|word| word.len() == 43).unwrap().to_owned()
    }

    #[tokio::test]
    async fn dang_ky_xac_minh_roi_doi_email() {
        TestDatabase::run(async |test| {
            let (app, services) = app_with(test, true).await;
            let body = json!({"email": "bob@example.com", "name": "Bob", "password": PASSWORD});
            let (status, _, created) = call(&app, "POST", "/register", None, Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            assert!(created["data"]["email_verified_at"].is_null());

            let body = json!({"token": token(&services)});
            let (status, _, verified) =
                call(&app, "POST", "/email-verification/confirm", None, Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{verified}");
            assert!(verified["data"]["email_verified_at"].is_string());

            let bob = bearer(&app, "bob@example.com", PASSWORD).await;
            let (status, _, me) = call(&app, "GET", "/me", Some(&bob), None).await;
            assert_eq!(status, StatusCode::OK);
            assert!(me["data"]["email_verified_at"].is_string());
            let (status, _, problem) =
                call(&app, "POST", "/me/email/verification", Some(&bob), None).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(problem["code"], "auth.email_already_verified");

            let body = json!({"new_email": "bob@new.example", "current_password": PASSWORD});
            let (status, _, _) = call(&app, "POST", "/me/email", Some(&bob), Some(body)).await;
            assert_eq!(status, StatusCode::ACCEPTED);
            let body = json!({"token": token(&services)});
            let (status, _, changed) =
                call(&app, "POST", "/email-verification/confirm", None, Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{changed}");
            assert_eq!(changed["data"]["email"], "bob@new.example");
        })
        .await
        .unwrap();
    }

    /// Đề bài giao diện W-059: người quản trị gửi lại thư xác minh cho một người dùng; người
    /// không có quyền bị từ chối; địa chỉ đã xác minh thì 409.
    #[tokio::test]
    async fn quan_tri_vien_gui_lai_thu_xac_minh() {
        TestDatabase::run(async |test| {
            let (app, services) = app_with(test, false).await;
            let root = crate::api::NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            crate::api::Accounts::create(services.users.as_ref(), &services.tenant, root)
                .await
                .unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;
            let ada_id = services
                .users
                .find_by_email(&services.tenant, "ada@example.com")
                .await
                .unwrap()
                .unwrap()
                .id;
            let path = format!("/users/{ada_id}/email/verification");
            let (status, _, _) = call(&app, "POST", &path, Some(&ada), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            let before = services.mails.sent.lock().unwrap().len();
            let (status, _, body) = call(&app, "POST", &path, Some(&admin), None).await;
            assert_eq!(status, StatusCode::ACCEPTED, "{body}");
            let sent = services.mails.sent.lock().unwrap().clone();
            assert_eq!(sent.len(), before + 1);
            assert_eq!(sent.last().unwrap().to, "ada@example.com");

            let body = json!({"token": token(&services)});
            let (status, _, _) =
                call(&app, "POST", "/email-verification/confirm", None, Some(body)).await;
            assert_eq!(status, StatusCode::OK);
            let (status, _, problem) = call(&app, "POST", &path, Some(&admin), None).await;
            assert_eq!(
                (status, problem["code"].as_str()),
                (StatusCode::CONFLICT, Some("auth.email_already_verified"))
            );
            let missing = format!("/users/{}/email/verification", uuid::Uuid::now_v7());
            let (status, _, _) = call(&app, "POST", &missing, Some(&admin), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        })
        .await
        .unwrap();
    }
}
