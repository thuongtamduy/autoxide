//! Shared foundation for connectors to real providers (step 70 P7, D-20261121-provider-accounts): one way
//! to build every payment, shipping, SMS and e-invoice connector…
//!
//! - **A provider account belongs to each tenant.** A connector declares each account field as a
//!   per-tenant setting in the `connection` group (`<module>.connection.<kind>.<field>`, secret fields
//!   with `.secret()` — see [`crate::tenancy::settings`]). A tenant sets / clears them with
//!   `PUT` / `DELETE /api/v1/kernel/settings/{name}` and tests them with `POST …/settings/{name}/test`
//!   ([`ConnectionProbe`](crate::connections::ConnectionProbe)). No environment variable exists for
//!   anything that belongs to a tenant.
//! - **Read the account at runtime** with [`ProviderAccounts`] (`context.require::<ProviderAccounts>()`):
//!   a required field the tenant has not set → [`not_configured`] (`provider.not_configured`); it never
//!   falls back to another tenant's account or the server's.
//! - **Incoming webhooks**: a connector declares a [`WebhookEndpoint`]; each tenant has its own URL ending in
//!   an unguessable identifier ([`ProviderAccounts::webhook_token`]). The module receiving the webhook finds
//!   the tenant from the identifier ([`ProviderAccounts::resolve_webhook`]) FIRST, then the connector
//!   verifies the signature with that tenant's secret. A tenant sees its URLs at
//!   `GET /api/v1/kernel/connections/webhooks`.
//! - **Outgoing calls** use [`http_client`] (no redirects, with a timeout; a tenant-chosen address
//!   may only be a public IP — D-20261121-ssrf-targets) and [`send`] (logs contain no secrets, plus metrics).
//!   Not to be called inside a write transaction.
//! - **Signatures**: [`hmac_sha256`], [`hex`], [`constant_time_eq`].
//!
//! Guide to writing a connector: `docs/guides/provider-adapters.md`.

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use axum::extract::State;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use crate::access::Principal;
use crate::error::{Error, ErrorKind};
use crate::mcp::Tool;
use crate::public_url::PublicUrl;
use crate::response::{Data, Problem};
use crate::secret::SecretKey;
use crate::tenancy::{
    DatabaseRegistry, SETTINGS_MANAGE, SettingValue, TenantContext, TenantId, TenantSetting,
    TenantSettings, link_token, resolve_link_token,
};

/// Purpose of the derived key for webhook identifiers.
const WEBHOOK_PURPOSE: &str = "kernel.provider_webhook";
/// Number of base64url characters in the raw part of a webhook identifier (32 characters = 192 bits).
const WEBHOOK_TOKEN_LEN: usize = 32;

/// One webhook path of a connector: the tenant pastes its URL into the provider's admin page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebhookEndpoint {
    connection: &'static str,
    path: &'static str,
    description: &'static str,
}

impl WebhookEndpoint {
    /// Webhook of connection `connection` (`<module>.connection.<kind>`) to `path` — the full path
    /// of the receiving route, WITHOUT the trailing identifier (e.g. `/api/v1/ledger/bank-feeds/sepay`);
    /// `description` (English) tells the tenant where to paste the URL at the provider.
    pub const fn new(
        connection: &'static str,
        path: &'static str,
        description: &'static str,
    ) -> Self {
        WebhookEndpoint { connection, path, description }
    }

    pub fn connection(&self) -> &'static str {
        self.connection
    }

    pub fn path(&self) -> &'static str {
        self.path
    }
}

/// The response the provider expects once the webhook is received (e.g. SePay needs `{"success": true}`).
/// A connector returns it through the trait's `webhook_ack`; if not declared, the receiving module returns
/// the standard envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebhookAck {
    /// HTTP status (200…299).
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl WebhookAck {
    /// Returns `status` with JSON body `body`.
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        WebhookAck { status, content_type: "application/json", body: body.into() }
    }

    /// Returns `204 No Content`.
    pub fn no_content() -> Self {
        WebhookAck { status: 204, content_type: "text/plain", body: String::new() }
    }
}

impl axum::response::IntoResponse for WebhookAck {
    fn into_response(self) -> axum::response::Response {
        let status = axum::http::StatusCode::from_u16(self.status)
            .ok()
            .filter(axum::http::StatusCode::is_success)
            .unwrap_or(axum::http::StatusCode::OK);
        if status == axum::http::StatusCode::NO_CONTENT {
            return status.into_response();
        }
        (status, [(axum::http::header::CONTENT_TYPE, self.content_type)], self.body).into_response()
    }
}

struct Inner {
    settings: Arc<dyn TenantSettings>,
    registry: DatabaseRegistry,
    secret: SecretKey,
    public_url: PublicUrl,
    webhooks: RwLock<Vec<WebhookEndpoint>>,
}

/// Per-tenant provider accounts: reads a tenant's connection settings and routes webhooks
/// back to the right tenant. Published by the kernel; connectors get it with
/// `context.require::<ProviderAccounts>()`. Cheap to clone.
#[derive(Clone)]
pub struct ProviderAccounts(Arc<Inner>);

impl ProviderAccounts {
    pub fn new(
        settings: Arc<dyn TenantSettings>,
        registry: DatabaseRegistry,
        secret: SecretKey,
        public_url: PublicUrl,
    ) -> Self {
        ProviderAccounts(Arc::new(Inner {
            settings,
            registry,
            secret,
            public_url,
            webhooks: RwLock::default(),
        }))
    }

    /// Context of tenant `id` — a connector's trait receives `TenantId`, and settings are read in that context.
    pub async fn tenant(&self, id: TenantId) -> Result<TenantContext, Error> {
        self.0.registry.tenant(id).await
    }

    /// The tenant's value of `setting`; `None` when the tenant has not set it (optional field).
    pub async fn get(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
    ) -> Result<Option<SettingValue>, Error> {
        self.0.settings.get(tenant, setting).await
    }

    /// The tenant's value of `setting`; the tenant has not set it → [`not_configured`]. Never replaced by
    /// another tenant's value or the server's.
    pub async fn require(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
    ) -> Result<SettingValue, Error> {
        self.get(tenant, setting).await?.ok_or_else(|| not_configured(setting.name()))
    }

    /// Declares a webhook path (call in the connector's `Module::start`). Declaring the same path again is ignored.
    pub fn register_webhook(&self, endpoint: WebhookEndpoint) {
        let mut known = self.0.webhooks.write().unwrap_or_else(PoisonError::into_inner);
        if !known.iter().any(|found| found.path == endpoint.path) {
            known.push(endpoint);
        }
    }

    fn endpoints(&self) -> Vec<WebhookEndpoint> {
        self.0.webhooks.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Raw part of the identifier: HMAC-SHA256 of `path` with the tenant's own derived key. Stored
    /// nowhere; the same tenant and path always yield the same identifier.
    async fn raw_token(&self, tenant: TenantId, path: &str) -> Result<String, Error> {
        let key = self.0.secret.derive_for(tenant, WEBHOOK_PURPOSE).await?;
        let mut encoded = URL_SAFE_NO_PAD.encode(hmac_sha256(&key, path.as_bytes()));
        encoded.truncate(WEBHOOK_TOKEN_LEN);
        Ok(encoded)
    }

    /// Webhook identifier of tenant `tenant` for path `path` (see [`WebhookEndpoint::new`]): the last segment
    /// of the URL the tenant pastes into the provider's page. Carries the tenant ID (except for the default
    /// tenant) in the form of [`link_token`], so the tenant can be found without any table lookup.
    pub async fn webhook_token(&self, tenant: &TenantContext, path: &str) -> Result<String, Error> {
        Ok(link_token(tenant, &self.raw_token(tenant.id(), path).await?))
    }

    /// Full webhook URL of tenant `tenant` for path `path` — for connectors that must send the notification URL
    /// in each request (e.g. `ipnUrl`, `CallbackUrl`). Built from `AUTOXIDE_HTTP_PUBLIC_URL`;
    /// if unset → `public_url_missing` (503). Never guessed from the request's `Host` header.
    pub async fn webhook_url(&self, tenant: &TenantContext, path: &str) -> Result<String, Error> {
        let token = self.webhook_token(tenant, path).await?;
        self.0.public_url.link(&format!("{path}/{token}"))
    }

    /// The tenant of a webhook to path `path` with identifier `token`. A wrong identifier, a missing tenant
    /// or one that cannot be served → [`webhook_unknown`] (the same error in every case). After this step the
    /// connector MUST still verify the signature with the secret of the tenant it found: the identifier is
    /// only for routing.
    pub async fn resolve_webhook(&self, path: &str, token: &str) -> Result<TenantContext, Error> {
        let Some((tenant, raw)) = resolve_link_token(&self.0.registry, None, token).await? else {
            return Err(webhook_unknown());
        };
        let expected = self.raw_token(tenant.id(), path).await?;
        if !constant_time_eq(expected.as_bytes(), raw.as_bytes()) {
            return Err(webhook_unknown());
        }
        // Webhook ghi dữ liệu: khách chỉ đọc từ chối (nhà cung cấp gửi lại sau), như mọi request ghi.
        if tenant.read_only() {
            return Err(crate::tenancy::errors::tenant_read_only());
        }
        Ok(tenant)
    }

    /// Every declared webhook path, with the URL of tenant `tenant`.
    async fn webhooks_of(&self, tenant: &TenantContext) -> Result<Vec<ConnectionWebhook>, Error> {
        let mut found = Vec::new();
        for endpoint in self.endpoints() {
            let token = self.webhook_token(tenant, endpoint.path).await?;
            let path = format!("{}/{token}", endpoint.path);
            found.push(ConnectionWebhook {
                connection: endpoint.connection.to_owned(),
                description: endpoint.description.to_owned(),
                url: self.0.public_url.link(&path).ok(),
                path,
            });
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(found)
    }
}

// ---- Môi trường thử theo khách ----------------------------------------------------------------------

/// Environment of a tenant's provider account (D-20261122-provider-environments): a test account only runs
/// against the provider's sandbox host. A connector with a sandbox host declares the tenant setting
/// `<module>.connection.<kind>.environment` with `.checked(Environment::check)`, reads it with
/// [`Environment::parse`], then chooses between TWO fixed operator-set addresses (`AUTOXIDE_<NAME>_API_URL`,
/// `AUTOXIDE_<NAME>_SANDBOX_API_URL`) — a tenant never types an address itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Environment {
    /// Default when the tenant does not set one.
    #[default]
    Production,
    Sandbox,
}

impl Environment {
    /// The value the tenant sets: `production` or `sandbox`; unset is `Production`. Any other value → `None`.
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim) {
            None | Some("production") => Some(Environment::Production),
            Some("sandbox") => Some(Environment::Sandbox),
            Some(_) => None,
        }
    }

    /// Validator for `TenantSetting::checked`.
    pub fn check(raw: &str) -> Result<(), crate::error::FieldError> {
        Environment::parse(Some(raw)).map(|_| ()).ok_or_else(|| {
            crate::error::FieldError::new("value", "invalid", "must be `production` or `sandbox`")
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Environment::Production => "production",
            Environment::Sandbox => "sandbox",
        }
    }

    /// Picks the address by environment.
    pub fn pick<'a>(self, production: &'a str, sandbox: &'a str) -> &'a str {
        match self {
            Environment::Production => production,
            Environment::Sandbox => sandbox,
        }
    }
}

// ---- Lỗi -----------------------------------------------------------------------------------------

/// `provider.not_configured` (422): the tenant has not set field `setting` of the provider account.
pub fn not_configured(setting: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "provider.not_configured",
        "This tenant has not set `{setting}`. Set it with `PUT /api/v1/kernel/settings/{setting}`; \
         `GET /api/v1/kernel/settings?group=connection` lists what a connection needs.",
    )
    .with_param("setting", setting)
}

/// `provider.webhook_unknown` (404): the identifier in the webhook URL belongs to no tenant.
pub fn webhook_unknown() -> Error {
    Error::not_found(
        "provider.webhook_unknown",
        "This webhook address is not known. Copy it again from \
         `GET /api/v1/kernel/connections/webhooks`.",
    )
}

// ---- Gọi ra ngoài --------------------------------------------------------------------------------

/// The address the connector's HTTP client calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A provider address set by the connector / operator (not chosen by the tenant).
    Fixed,
    /// An address chosen by the tenant (tenant setting): connects only to public IPs, unless the operator
    /// allows private networks (`allow_private`). That setting must also be `.checked(…)` with
    /// [`check_target_url`](crate::net::check_target_url) when saved.
    TenantChosen { allow_private: bool },
}

/// HTTP client for connectors: has a timeout and does not follow redirects (sending a secret to an address
/// other than the configured one is a risk), with Autoxide's `User-Agent`.
pub fn http_client(timeout: Duration, target: Target) -> Result<reqwest::Client, Error> {
    crate::tls::install_crypto_provider();
    let mut builder = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Autoxide/", env!("CARGO_PKG_VERSION")));
    if target == (Target::TenantChosen { allow_private: false }) {
        builder = builder.dns_resolver(crate::net::PublicOnly);
    }
    builder.build().map_err(|error| Error::internal(format!("provider HTTP client: {error}")))
}

/// The provider's response: HTTP status and raw body.
#[derive(Debug, Clone)]
pub struct ProviderReply {
    pub status: u16,
    pub body: bytes::Bytes,
}

impl ProviderReply {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Body read as JSON; `None` when it is not JSON.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// The provider could not be reached (network error, timeout, truncated body): temporary, can be retried. A
/// connector converts it to the `…provider_unavailable` error of the contract it implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreachable;

/// Sends `request` to provider `provider` (the connector's stable name) for task `purpose` (a fixed label,
/// e.g. `create_payment`). Logs (no URL with parameters, no headers, no body — only the purpose and HTTP
/// status) and records the metric `kernel_provider_calls_total`. A status outside 2xx is still `Ok`: the
/// connector reads the error itself according to the provider's documentation.
pub async fn send(
    request: reqwest::RequestBuilder,
    provider: &'static str,
    purpose: &'static str,
) -> Result<ProviderReply, Unreachable> {
    let outcome = async {
        let response = request.send().await?;
        let status = response.status().as_u16();
        Ok::<_, reqwest::Error>(ProviderReply { status, body: response.bytes().await? })
    }
    .await;
    match outcome {
        Ok(reply) => {
            let class = if reply.is_success() { "ok" } else { "refused" };
            metrics::counter!("kernel_provider_calls_total", "provider" => provider, "outcome" => class)
                .increment(1);
            if !reply.is_success() {
                tracing::warn!(provider, purpose, status = reply.status, "provider refused a call");
            }
            Ok(reply)
        }
        Err(error) => {
            metrics::counter!("kernel_provider_calls_total", "provider" => provider, "outcome" => "unreachable")
                .increment(1);
            // `without_url`: URL có thể mang tham số nhạy cảm.
            tracing::warn!(provider, purpose, error = %error.without_url(), "provider unreachable");
            Err(Unreachable)
        }
    }
}

// ---- Chữ ký --------------------------------------------------------------------------------------

/// HMAC-SHA256 of `data` with `key`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    // HMAC nhận khoá dài bất kỳ: `new_from_slice` không lỗi được; nhánh lỗi trả chữ ký của khoá rỗng
    // để không cần `unwrap`.
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .unwrap_or_else(|_| <Hmac<Sha256> as KeyInit>::new(&Default::default()));
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// `bytes` as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// Compares two byte strings in time independent of the position of the first difference (for signatures, API keys).
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---- API: URL webhook của khách ------------------------------------------------------------------

/// A webhook address of this tenant at a partner connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct ConnectionWebhook {
    /// The connection, `<module>.connection.<kind>`.
    #[schema(example = "sepay.connection.bank")]
    connection: String,
    /// Where to paste the address at the partner.
    description: String,
    /// Path of the address; its last part identifies this tenant and cannot be guessed.
    #[schema(example = "/api/v1/ledger/bank-feeds/sepay/Zk3v…")]
    path: String,
    /// Full address, when the server has `AUTOXIDE_HTTP_PUBLIC_URL`; otherwise put the server's
    /// public address in front of `path`.
    url: Option<String>,
}

/// List the webhook addresses of this tenant.
///
/// Requires `kernel.settings.manage`. One line per webhook an installed partner connection
/// receives (bank transactions, payment results, delivery updates…). Each address is this tenant's
/// own: paste it into the partner's settings page. The partner's notifications are still checked
/// with this tenant's secret (signature or API key) before anything is stored.
#[utoipa::path(
    get,
    path = "/connections/webhooks",
    extensions(("x-permissions" = json!(["kernel.settings.manage"]))),
    operation_id = "list_connection_webhooks",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The webhook addresses, sorted by path.", body = Data<Vec<ConnectionWebhook>>),
        (status = 401, description = "Not signed in; code `unauthorized` or the code of the authentication module.", body = Problem),
        (status = 403, description = "Missing permission `kernel.settings.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_connection_webhooks(
    State(accounts): State<ProviderAccounts>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<ConnectionWebhook>>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: accounts.webhooks_of(&tenant).await? })
}

pub(crate) fn routes(accounts: ProviderAccounts) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(list_connection_webhooks)).with_state(accounts)
}

/// No input.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListConnectionWebhooks {}

pub(crate) fn tools(accounts: ProviderAccounts) -> Vec<Tool> {
    vec![
        Tool::new(
            "list_connection_webhooks",
            "List the webhook addresses of the current tenant: one per webhook an installed \
             partner connection receives (bank transactions, payment results, delivery \
             updates…), to paste into the partner's settings page.",
            move |tenant: TenantContext, _principal: Principal, _input: ListConnectionWebhooks| {
                let accounts = accounts.clone();
                async move { accounts.webhooks_of(&tenant).await }
            },
        )
        .requires(SETTINGS_MANAGE)
        .read_only(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_theo_vector_rfc_4231() {
        // RFC 4231, test case 2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(hex(&mac), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn moi_truong_cua_tai_khoan() {
        assert_eq!(Environment::parse(None), Some(Environment::Production));
        assert_eq!(Environment::parse(Some(" sandbox ")), Some(Environment::Sandbox));
        assert_eq!(Environment::parse(Some("test")), None);
        assert!(Environment::check("production").is_ok() && Environment::check("prod").is_err());
        assert_eq!(Environment::Sandbox.pick("https://a", "https://b"), "https://b");
        assert_eq!(Environment::default().as_str(), "production");
    }

    #[test]
    fn so_sanh_thoi_gian_hang() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn ack_chi_tra_ma_thanh_cong() {
        use axum::response::IntoResponse;
        assert_eq!(WebhookAck::json(201, "{}").into_response().status(), 201);
        assert_eq!(WebhookAck::json(500, "{}").into_response().status(), 200);
        assert_eq!(WebhookAck::no_content().into_response().status(), 204);
    }
}

// Thử phá hàm so chữ ký (D-20261123-property-fuzzing).
#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn so_sanh_thoi_gian_hang_giong_so_sanh_thuong(
            a in proptest::collection::vec(any::<u8>(), 0..64),
            b in proptest::collection::vec(any::<u8>(), 0..64),
        ) {
            prop_assert_eq!(constant_time_eq(&a, &b), a == b);
            prop_assert!(constant_time_eq(&a, &a.clone()));
        }

        #[test]
        fn hex_dai_gap_doi_va_chi_co_chu_thuong(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let text = hex(&bytes);
            prop_assert_eq!(text.len(), bytes.len() * 2);
            prop_assert!(text.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')));
        }
    }
}
