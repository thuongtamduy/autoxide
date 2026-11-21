//! `POST /api-keys/{id}/rotate`: thay một API key bằng key mới mà máy không mất lúc nào (bước 54).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use super::api_keys::CreatedApiKeyResponse;
use crate::api_key::{Reauth, Rotation};

/// Số giờ key cũ còn chạy khi không nêu.
pub(crate) const DEFAULT_OVERLAP_HOURS: u32 = 24;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new().routes(routes!(rotate_api_key))
}

/// How to replace a key.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct RotateApiKeyRequest {
    /// Hours the old key keeps working, 0 to 72 (24 by default): time to switch every program
    /// to the new key. 0 stops the old key at once.
    #[schema(example = 24)]
    overlap_hours: Option<u32>,
    /// When the new key stops working (RFC 3339), at most `AUTOXIDE_AUTH_API_KEY_MAX_DAYS` days
    /// ahead. Omit for that longest lifetime.
    expires_at: Option<Timestamp>,
    /// Your current password, as when creating a key. Or `totp_code`, a passkey
    /// (`passkey_ceremony_id` and `passkey_credential`) or a `reauth_token` instead — exactly one.
    #[serde(default)]
    current_password: String,
    /// A code from your authenticator app, instead of the password.
    #[schema(example = "123456")]
    totp_code: Option<String>,
    /// One of your passkeys, instead of the password (see `POST /api/v1/auth/api-keys`).
    passkey_ceremony_id: Option<Uuid>,
    /// The browser's answer for `passkey_ceremony_id`.
    #[schema(value_type = Option<Object>)]
    passkey_credential: Option<serde_json::Value>,
    /// A confirmation token from `POST /api/v1/auth/me/reauth`, instead of the password.
    reauth_token: Option<String>,
}

impl From<RotateApiKeyRequest> for Rotation {
    fn from(input: RotateApiKeyRequest) -> Self {
        Rotation {
            overlap_hours: input.overlap_hours.unwrap_or(DEFAULT_OVERLAP_HOURS),
            expires_at: input.expires_at,
            current_password: input.current_password,
            second_factor: match (input.totp_code, input.passkey_ceremony_id) {
                _ if input.reauth_token.is_some() => input.reauth_token.map(Reauth::Token),
                (Some(code), _) => Some(Reauth::Totp(code)),
                (None, Some(ceremony_id)) => Some(Reauth::Passkey {
                    ceremony_id,
                    credential: input.passkey_credential.unwrap_or_default(),
                }),
                (None, None) => None,
            },
        }
    }
}

/// Replace one of your API keys with a new one.
///
/// The new key has the same name, permissions, organization unit, allowed addresses and kind;
/// its `rotated_from` names the old key. The old key keeps working for `overlap_hours` (24 by
/// default), then expires: switch your programs to the new key in that time. Confirm it is you
/// like when creating a key. Signed in with a password session only: an API key cannot rotate
/// keys. The response carries the new key, shown only once.
#[utoipa::path(
    post,
    path = "/api-keys/{id}/rotate",
    extensions(("x-permissions" = json!([]))),
    operation_id = "rotate_api_key",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the key to replace.")),
    request_body = RotateApiKeyRequest,
    responses(
        (status = 201, description = "The new key; `key` holds its secret.", body = Data<CreatedApiKeyResponse>),
        (status = 400, description = "The current password is wrong; code `auth.wrong_password`.", body = Problem),
        (status = 401, description = "Not signed in, or the passkey answer is not valid; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.api_key_cannot_create_keys`.", body = Problem),
        (status = 404, description = "You have no key with this id; code `auth.api_key_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid, e.g. `overlap_hours` above 72, an `expires_at` too far ahead, or a permission of the old key you no longer have (`not_granted`); see `errors`.", body = Problem),
        (status = 429, description = "Too many wrong passwords or codes; code `auth.login_throttled`.", body = Problem),
    )
)]
async fn rotate_api_key(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RotateApiKeyRequest>,
) -> Result<Created<CreatedApiKeyResponse>, Error> {
    let created = state.api_keys.rotate(&tenant, &principal, id, input.into()).await?;
    Ok(Created::new(format!("/auth/api-keys/{}", created.key.id), created.into()))
}

#[cfg(test)]
mod tests {
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::api::{Accounts, NewAccount};
    use crate::http::test_support::{PASSWORD, app, bearer, call};

    // Bước 54: xoay vòng không gián đoạn — key cũ còn chạy trong khoảng chồng, key mới giữ nguyên
    // cấu hình và ghi key nó thay; `overlap_hours = 0` thì key cũ thôi chạy ngay.
    #[tokio::test]
    async fn xoay_vong_key_khong_gian_doan() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(services.users.as_ref(), &services.tenant, root).await.unwrap();
            let session = bearer(&app, "root@example.com", PASSWORD).await;
            let body = json!({"name": "nightly", "kind": "agent", "permissions": ["auth.users.read"], "current_password": PASSWORD});
            let (status, _, first) = call(&app, "POST", "/api-keys", Some(&session), Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{first}");
            let first_id = first["data"]["id"].as_str().unwrap().to_owned();
            let first_key = format!("Bearer {}", first["data"]["key"].as_str().unwrap());

            let rotate = |id: &str| format!("/api-keys/{id}/rotate");
            let confirm = json!({"current_password": PASSWORD});
            let (status, _, second) =
                call(&app, "POST", &rotate(&first_id), Some(&session), Some(confirm.clone())).await;
            assert_eq!(status, StatusCode::CREATED, "{second}");
            let data = &second["data"];
            assert_eq!(data["rotated_from"], json!(first_id));
            assert_eq!(data["name"], "nightly");
            assert_eq!(data["kind"], "agent");
            assert_eq!(data["permissions"], json!(["auth.users.read"]));
            assert_ne!(data["key"], first["data"]["key"]);
            let second_id = data["id"].as_str().unwrap().to_owned();
            let second_key = format!("Bearer {}", data["key"].as_str().unwrap());
            // Trong khoảng chồng: cả hai key cùng chạy.
            for key in [&first_key, &second_key] {
                let (status, _, users) = call(&app, "GET", "/users", Some(key), None).await;
                assert_eq!(status, StatusCode::OK, "{users}");
            }

            // Key không xoay được key (như không tạo được key).
            let (status, _, nested) =
                call(&app, "POST", &rotate(&second_id), Some(&second_key), Some(confirm.clone())).await;
            assert_eq!((status, nested["code"].as_str()), (StatusCode::FORBIDDEN, Some("auth.api_key_cannot_create_keys")));
            // Quá 72 giờ chồng, key không có, mật khẩu sai.
            let long = json!({"current_password": PASSWORD, "overlap_hours": 73});
            let (status, _, problem) = call(&app, "POST", &rotate(&second_id), Some(&session), Some(long)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(problem["errors"][0]["field"], "overlap_hours");
            let missing = rotate("0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11");
            let (status, _, _) = call(&app, "POST", &missing, Some(&session), Some(confirm.clone())).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            let wrong = json!({"current_password": "wrong one"});
            let (status, _, _) = call(&app, "POST", &rotate(&second_id), Some(&session), Some(wrong)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);

            // `overlap_hours = 0`: key cũ thôi chạy ngay, key mới chạy.
            let now = json!({"current_password": PASSWORD, "overlap_hours": 0});
            let (status, _, third) = call(&app, "POST", &rotate(&second_id), Some(&session), Some(now)).await;
            assert_eq!(status, StatusCode::CREATED, "{third}");
            let (status, _, expired) = call(&app, "GET", "/users", Some(&second_key), None).await;
            assert_eq!((status, expired["code"].as_str()), (StatusCode::UNAUTHORIZED, Some("auth.api_key_expired")));
            let third_key = format!("Bearer {}", third["data"]["key"].as_str().unwrap());
            let (status, _, _) = call(&app, "GET", "/users", Some(&third_key), None).await;
            assert_eq!(status, StatusCode::OK);
        })
        .await
        .unwrap();
    }
}
