//! Endpoint đặt lại mật khẩu qua mail.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::response::Problem;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use super::HttpState;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new().routes(routes!(request_reset)).routes(routes!(confirm_reset))
}

/// The account whose password to reset.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct ResetRequest {
    #[validate(length(min = 1, max = 254))]
    #[schema(example = "ada@example.com")]
    email: String,
}

/// A new password, with the token from the reset mail.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct ResetConfirmation {
    /// The token from the reset mail.
    token: String,
    /// 12 to 128 characters.
    password: String,
}

/// Ask for a password-reset mail.
///
/// Always answers 202, whether or not an account has this email, and in the same time:
/// the account is looked up and the mail sent by a background job a moment later, so
/// neither the answer nor its timing reveals who has an account. When one does, a mail
/// with a reset token (valid 30 minutes) is sent in the language of `Accept-Language`. At
/// most one mail per minute per account.
#[utoipa::path(
    post,
    path = "/password-reset",
    extensions(("x-public" = json!(true))),
    operation_id = "request_password_reset",
    request_body = ResetRequest,
    responses(
        (status = 202, description = "If an active account has this email, a reset mail is on its way."),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
    )
)]
async fn request_reset(
    State(state): State<Arc<HttpState>>,
    language: Language,
    ValidJson(input): ValidJson<ResetRequest>,
) -> Result<StatusCode, Error> {
    state.resets.request(&input.email, language).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Choose a new password with the token from the reset mail.
///
/// Signs the account out everywhere; every other reset token of the account stops
/// working. Then sign in with the new password.
#[utoipa::path(
    post,
    path = "/password-reset/confirm",
    extensions(("x-public" = json!(true))),
    operation_id = "confirm_password_reset",
    request_body = ResetConfirmation,
    responses(
        (status = 204, description = "The password was changed."),
        (status = 400, description = "The token is unknown, expired or used; code `auth.invalid_reset_token`, with `reason` (`expired`, `used` or `unknown`). Ask for a new mail with `POST /api/v1/auth/password-reset`.", body = Problem),
        (status = 422, description = "The password does not follow the rules (12 to 128 characters); see `errors`.", body = Problem),
    )
)]
async fn confirm_reset(
    State(state): State<Arc<HttpState>>,
    ValidJson(input): ValidJson<ResetConfirmation>,
) -> Result<StatusCode, Error> {
    state.resets.confirm(&input.token, input.password).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::http::test_support::{app, call};

    #[tokio::test]
    async fn yeu_cau_dat_lai_luon_tra_202() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;

            let known = json!({"email": "ada@example.com"});
            let unknown = json!({"email": "ghost@example.com"});
            let (status_known, _, _) =
                call(&app, "POST", "/password-reset", None, Some(known)).await;
            let (status_unknown, _, _) =
                call(&app, "POST", "/password-reset", None, Some(unknown)).await;

            // Cùng một câu trả lời: không ai biết email nào có tài khoản. Request chỉ xếp job
            // (D-162): mail đi khi worker chạy.
            assert_eq!(status_known, StatusCode::ACCEPTED);
            assert_eq!(status_unknown, StatusCode::ACCEPTED);
            assert!(services.mails.sent.lock().unwrap().is_empty());
            crate::testing::send_resets(&services).await;
            assert_eq!(services.mails.sent.lock().unwrap().len(), 1);

            let body = json!({"token": "nope", "password": "a brand new passphrase"});
            let (status, _, problem) =
                call(&app, "POST", "/password-reset/confirm", None, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(problem["code"], "auth.invalid_reset_token");
        })
        .await
        .unwrap();
    }
}
