//! Endpoint xác minh email và đổi email.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::response::{Data, Problem};
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use super::HttpState;
use super::users::UserResponse;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(resend_verification))
        .routes(routes!(change_email))
        .routes(routes!(confirm_email))
}

/// A new email address for the signed-in account.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct ChangeEmailRequest {
    /// The new address. Case and surrounding spaces do not matter.
    #[validate(length(min = 1, max = 254))]
    #[schema(example = "ada@example.org")]
    new_email: String,
    #[validate(length(min = 1, max = 1024))]
    current_password: String,
}

/// The token from a verification mail.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct VerificationConfirmation {
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
    principal: Principal,
    language: Language,
) -> Result<StatusCode, Error> {
    state.verifications.send_current(principal.user_id(), language).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Change your email address.
///
/// Needs the current password and a session signed in with a password (an API key
/// cannot do this). A verification mail goes to the NEW address; the email changes
/// only when its token is confirmed with `POST /api/v1/auth/email-verification/confirm`,
/// within 24 hours. Until then the old address stays in use. Wrong passwords count
/// towards the sign-in lock of the email.
#[utoipa::path(
    post,
    path = "/me/email",
    operation_id = "change_email",
    security(("bearer" = [])),
    request_body = ChangeEmailRequest,
    responses(
        (status = 202, description = "A verification mail is on its way to the new address."),
        (status = 400, description = "The current password is wrong; code `auth.wrong_password`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Called with an API key (code `auth.session_required`), or the account is disabled (code `auth.user_disabled`).", body = Problem),
        (status = 409, description = "Another account has this email (code `auth.email_taken`), or it is already your verified address (code `auth.email_already_verified`).", body = Problem),
        (status = 422, description = "The body is missing a field or the address is invalid; see `errors`.", body = Problem),
        (status = 429, description = "Too many wrong passwords for this email; code `auth.login_throttled`. Wait the number of seconds in the `Retry-After` header.", body = Problem),
    )
)]
async fn change_email(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    language: Language,
    ValidJson(input): ValidJson<ChangeEmailRequest>,
) -> Result<StatusCode, Error> {
    super::require_session(&principal)?;
    state
        .verifications
        .change_email(principal.user_id(), &input.current_password, &input.new_email, language)
        .await?;
    Ok(StatusCode::ACCEPTED)
}

/// Confirm an email address with the token from a verification mail.
///
/// Needs no sign-in: the token proves the mail was received. When the token was sent
/// for a new address (`POST /api/v1/auth/me/email`), the account's email becomes that
/// address; sign in with it from now on. Every other verification token of the
/// account stops working.
#[utoipa::path(
    post,
    path = "/email-verification/confirm",
    operation_id = "confirm_email_verification",
    request_body = VerificationConfirmation,
    responses(
        (status = 200, description = "Verified; the account as it is now.", body = Data<UserResponse>),
        (status = 400, description = "The token is unknown, expired or used; code `auth.invalid_verification_token`.", body = Problem),
        (status = 409, description = "Another account took this address meanwhile; code `auth.email_taken`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
    )
)]
async fn confirm_email(
    State(state): State<Arc<HttpState>>,
    ValidJson(input): ValidJson<VerificationConfirmation>,
) -> Result<Data<UserResponse>, Error> {
    let user = state.verifications.confirm(&input.token).await?;
    Ok(Data { data: user.into() })
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
}
