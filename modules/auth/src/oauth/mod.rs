//! Máy chủ uỷ quyền OAuth 2.1 cho client MCP (bước 71 G4, D-20261126-mcp-oauth).
//!
//! Luồng: client MCP gặp 401 ở `/mcp` → đọc Protected Resource Metadata (kernel) → metadata của máy chủ này
//! ([`OAuthService::metadata`]) → nhận diện bằng tài liệu metadata của nó (CIMD, `client_id` là URL https) hay đăng
//! ký động (RFC 7591) → mở trình duyệt ở `/authorize` (PKCE `S256` bắt buộc) → trang consent của giao diện gọi API
//! của module này để người dùng duyệt / từ chối → client đổi mã lấy access token + refresh token ở `/token`.
//!
//! - Token opaque (`axo1_` access, `axr1_` refresh, mã `axc1_`), mang ID khách, chỉ lưu bản băm: bộ xác thực phân
//!   luồng theo tiền tố, không lẫn với access token của phiên.
//! - Mỗi lần duyệt là một **grant**: người dùng, client, resource, danh sách permission cụ thể. Quyền của token =
//!   permission của grant ∩ quyền hiện tại của người dùng, đọc lại mỗi request: thu hồi có hiệu lực ngay.
//! - Lỗi của `/token`, `/revoke`, `/register` theo RFC 6749 §5.2 / RFC 7591 §3.2.2 ([`OAuthError`]), không phải
//!   Problem: client OAuth chỉ đọc dạng đó.

mod clients;
mod consent;
pub(crate) mod entity;
mod grants;
#[cfg(test)]
mod tests;
mod tokens;

use std::sync::Arc;

use autoxide_kernel::access::{PermissionCatalog, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantContext, TenantId};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use jiff::SignedDuration;
use serde_json::{Map, Value, json};

pub(crate) use self::clients::{
    ClientDirectory, RegisteredClient, Registration, RegistrationResponse,
};
pub(crate) use self::consent::{AuthorizeQuery, Decision, DecisionResult, RequestView};
pub(crate) use self::grants::{GrantView, revoke_user_grants_in};
pub(crate) use self::tokens::{ACCESS_PREFIX, RevokeForm, TokenForm, TokenResponse};
use crate::guardrail::GuardrailService;
use crate::role::RoleService;

/// Đường dẫn của các endpoint, dưới `/api/v1/auth` (metadata ghép với issuer).
pub(crate) const AUTHORIZE_PATH: &str = "/api/v1/auth/oauth/authorize";
pub(crate) const TOKEN_PATH: &str = "/api/v1/auth/oauth/token";
pub(crate) const REVOKE_PATH: &str = "/api/v1/auth/oauth/revoke";
pub(crate) const REGISTER_PATH: &str = "/api/v1/auth/oauth/register";

/// Đăng ký động mỗi địa chỉ mỗi giờ.
const REGISTRATIONS_PER_HOUR: u32 = 10;

/// Cấu hình đã đọc (`AUTOXIDE_AUTH_OAUTH_*`).
#[derive(Debug, Clone)]
pub(crate) struct OAuthConfig {
    /// Trang consent của giao diện, có `{token}`.
    pub consent_url: String,
    pub dynamic_registration: bool,
    pub allow_private_networks: bool,
    pub grant_days: i64,
    pub access_ttl: SignedDuration,
}

/// Máy chủ uỷ quyền. Có mặt chỉ khi bật (có public URL không path và trang consent).
pub(crate) struct OAuthService {
    pub config: OAuthConfig,
    /// Issuer: public URL dạng chuẩn, không path (giống hệt ở metadata, PRM và `iss`).
    pub issuer: String,
    pub databases: DatabaseRegistry,
    pub roles: Arc<RoleService>,
    pub guardrails: Arc<GuardrailService>,
    pub catalog: PermissionCatalog,
    pub outbox: Outbox,
    pub clients: ClientDirectory,
}

impl OAuthService {
    /// Dựng khi đủ điều kiện; `None` khi OAuth tắt (thiếu trang consent hay public URL, public URL có path).
    pub(crate) fn enabled(
        config: Option<OAuthConfig>,
        public_url: Option<&str>,
        databases: DatabaseRegistry,
        roles: Arc<RoleService>,
        guardrails: Arc<GuardrailService>,
        catalog: PermissionCatalog,
        outbox: Outbox,
    ) -> Result<Option<Self>, Error> {
        let (Some(config), Some(public_url)) = (config, public_url) else { return Ok(None) };
        let Some(issuer) = autoxide_kernel::access::canonical_resource(public_url) else {
            return Ok(None);
        };
        if issuer.split_once("://").is_some_and(|(_, rest)| rest.contains('/')) {
            // Kernel cũng tắt phần resource server và ghi cảnh báo.
            return Ok(None);
        }
        let clients = ClientDirectory::new(config.allow_private_networks, issuer.clone())?;
        Ok(Some(OAuthService {
            config,
            issuer,
            databases,
            roles,
            guardrails,
            catalog,
            outbox,
            clients,
        }))
    }

    /// Khách của issuer (nền tảng): nơi lưu client đăng ký động và yêu cầu uỷ quyền đang chờ.
    pub(crate) async fn platform(&self) -> Result<TenantContext, Error> {
        self.databases.tenant(autoxide_kernel::tenancy::DEFAULT_TENANT).await
    }

    /// `POST /oauth/register` (RFC 7591): hạn mức theo địa chỉ, rồi lưu client.
    pub(crate) async fn register(
        &self,
        input: Registration,
        ip: Option<std::net::IpAddr>,
    ) -> Result<RegisteredClient, OAuthError> {
        let platform = self.platform().await?;
        if let Some(ip) = ip {
            let throttle = autoxide_kernel::throttle::Throttle::new(
                "auth",
                REGISTRATIONS_PER_HOUR,
                std::time::Duration::from_secs(3600),
            )?;
            let key = autoxide_kernel::throttle::address_key("oauth_register", ip);
            if !throttle.hit(&platform, &key).await?.is_allowed() {
                return Err(OAuthError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "slow_down",
                    "Too many client registrations from this address; retry in an hour.",
                ));
            }
        }
        clients::register(&platform, input).await
    }

    /// Các resource máy chủ này cấp token cho: `/mcp`, `/api/v1` (và `/mcp/<module>` dưới `/mcp`).
    pub(crate) fn resource_of(&self, raw: Option<&str>) -> Option<String> {
        let mcp = format!("{}/mcp", self.issuer);
        let Some(raw) = raw.filter(|raw| !raw.trim().is_empty()) else { return Some(mcp) };
        let resource = autoxide_kernel::access::canonical_resource(raw)?;
        let rest = resource.strip_prefix(&mcp);
        let module = rest.and_then(|rest| rest.strip_prefix('/'));
        let valid = rest == Some("")
            || module.is_some_and(|module| {
                !module.is_empty()
                    && module.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte == b'_' || byte.is_ascii_digit()
                    })
            })
            || resource == format!("{}/api/v1", self.issuer);
        valid.then_some(resource)
    }

    /// Metadata RFC 8414 (kernel đặt `issuer`).
    pub(crate) fn metadata(&self, issuer: &str) -> Map<String, Value> {
        let mut scopes: Vec<String> = self
            .catalog
            .iter()
            .filter_map(|permission| {
                permission.name().split_once('.').map(|(module, _)| format!("{module}.*"))
            })
            .collect();
        scopes.sort();
        scopes.dedup();
        scopes.push("offline_access".to_owned());
        let mut document = json!({
            "authorization_endpoint": format!("{issuer}{AUTHORIZE_PATH}"),
            "token_endpoint": format!("{issuer}{TOKEN_PATH}"),
            "revocation_endpoint": format!("{issuer}{REVOKE_PATH}"),
            "response_types_supported": ["code"],
            "response_modes_supported": ["query"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "revocation_endpoint_auth_methods_supported": ["none"],
            "client_id_metadata_document_supported": true,
            "authorization_response_iss_parameter_supported": true,
            "scopes_supported": scopes,
            "service_documentation": format!("{issuer}/llms.txt"),
        });
        if self.config.dynamic_registration
            && let Value::Object(object) = &mut document
        {
            object.insert(
                "registration_endpoint".to_owned(),
                json!(format!("{issuer}{REGISTER_PATH}")),
            );
        }
        match document {
            Value::Object(object) => object,
            _ => Map::new(),
        }
    }

    /// Khách ghi trong một token (`<tiền tố><32 chữ hex>_<ngẫu nhiên>`), khi đúng dạng.
    pub(crate) fn tenant_in(token: &str, prefix: &str) -> Option<TenantId> {
        let rest = token.strip_prefix(prefix)?;
        let hex = rest.get(..32)?;
        let random = rest.get(32..)?.strip_prefix('_')?;
        if random.is_empty() || !hex.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        uuid::Uuid::try_parse(hex).ok().map(TenantId::from_uuid)
    }
}

/// Bộ xác thực công bố cho kernel (metadata của máy chủ uỷ quyền).
pub(crate) struct Metadata(pub Arc<OAuthService>);

impl autoxide_kernel::access::AuthorizationServer for Metadata {
    fn metadata(&self, issuer: &str) -> Map<String, Value> {
        self.0.metadata(issuer)
    }
}

/// Lỗi theo RFC 6749 §5.2 (và RFC 7591 §3.2.2 cho đăng ký, RFC 7009 cho thu hồi): `{error, error_description}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OAuthError {
    pub status: StatusCode,
    pub error: &'static str,
    pub description: String,
}

impl OAuthError {
    pub(crate) fn new(
        status: StatusCode,
        error: &'static str,
        description: impl Into<String>,
    ) -> Self {
        OAuthError { status, error, description: description.into() }
    }

    pub(crate) fn invalid_request(description: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", description)
    }

    pub(crate) fn invalid_grant(description: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_grant", description)
    }

    pub(crate) fn invalid_client(description: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "invalid_client", description)
    }

    pub(crate) fn unavailable() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "invalid_request",
            "OAuth sign-in is not set up on this server; connect with an API key instead.",
        )
    }
}

/// Lỗi chung của service (database…) thành `server_error` (chi tiết vào log, không ra ngoài).
impl From<Error> for OAuthError {
    fn from(error: Error) -> Self {
        match error.kind() {
            autoxide_kernel::error::ErrorKind::Unavailable => OAuthError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily_unavailable",
                error.message(),
            ),
            autoxide_kernel::error::ErrorKind::TooManyRequests => {
                OAuthError::new(StatusCode::TOO_MANY_REQUESTS, "slow_down", error.message())
            }
            _ => {
                tracing::error!(%error, "OAuth endpoint failed");
                OAuthError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "server_error",
                    "Internal error.",
                )
            }
        }
    }
}

impl From<sea_orm::DbErr> for OAuthError {
    fn from(error: sea_orm::DbErr) -> Self {
        OAuthError::from(Error::from(error))
    }
}

/// Error answer of the OAuth endpoints (RFC 6749 §5.2, RFC 7591 §3.2.2).
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
#[schema(as = OAuthErrorResponse)]
pub(crate) struct ErrorBody {
    /// Machine-readable error, e.g. `invalid_grant`, `invalid_request`, `invalid_client_metadata`.
    pub error: &'static str,
    /// What went wrong, for the developer.
    pub error_description: String,
}

impl IntoResponse for OAuthError {
    fn into_response(self) -> Response {
        let body = ErrorBody { error: self.error, error_description: self.description };
        let status = self.status;
        let mut response = (status, axum::Json(body)).into_response();
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if status == StatusCode::UNAUTHORIZED {
            headers.insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"oauth\""),
            );
        }
        // Lớp chuẩn hoá lỗi của kernel giữ nguyên thân này (không đổi sang Problem).
        response.extensions_mut().insert(autoxide_kernel::http::StandardErrorBody);
        response
    }
}

/// Dấu vân tay của một token OAuth (mã, access token, refresh token) để lưu và tra trong database: chỉ lưu dấu này,
/// không lưu token, nên lộ bảng không dùng được token.
///
/// Vì sao SHA-256 là đủ, không cần hàm băm chậm (Argon2) như mật khẩu: token do server sinh bằng 256 bit ngẫu nhiên
/// (`random_token(32)`), không đoán hay dò từ điển được; hàm chậm chỉ cần cho bí mật do người chọn. Tra theo dấu
/// bằng điều kiện bằng của database (index), không so chuỗi trong bộ nhớ, nên không có rò rỉ thời gian theo từng ký tự.
pub(crate) fn token_digest(token: &str) -> String {
    autoxide_kernel::secret::sha256(token)
}

/// Người duyệt / quản lý grant phải là phiên đăng nhập của người làm nhân sự (không API key, không token OAuth).
pub(crate) fn require_person(principal: &Principal) -> Result<(), Error> {
    crate::access::require_session(principal)?;
    // Cho phép client làm thay mình là việc của chính người đó, trong khách của mình: không qua uỷ quyền, hỗ trợ hay
    // nhân sự nền tảng vào khách khác.
    if principal.realm() == autoxide_kernel::access::Realm::Customer
        || principal.operator_from().is_some()
        || !matches!(principal.access(), autoxide_kernel::access::AccessPath::Direct)
    {
        return Err(Error::forbidden(
            "auth.oauth_staff_only",
            "Only staff working directly in their own tenant can connect MCP clients.",
        ));
    }
    Ok(())
}

/// 404 `auth.oauth_disabled`: OAuth tắt trên máy chủ này.
pub(crate) fn disabled() -> Error {
    Error::not_found(
        "auth.oauth_disabled",
        "OAuth sign-in for MCP clients is off on this server: set AUTOXIDE_HTTP_PUBLIC_URL and \
         AUTOXIDE_AUTH_OAUTH_CONSENT_URL, or connect with an API key.",
    )
}
