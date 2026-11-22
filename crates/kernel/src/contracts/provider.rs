//! Nền chung cho bộ nối nhà cung cấp thật (bước 70 P7, D-20261121-provider-accounts): một cách
//! làm cho mọi bộ nối thanh toán, vận chuyển, SMS, hoá đơn điện tử…
//!
//! - **Tài khoản nhà cung cấp là của từng khách.** Bộ nối khai từng trường của tài khoản là một
//!   cấu hình theo khách thuộc nhóm `connection` (`<module>.connection.<loại>.<trường>`, trường bí
//!   mật `.secret()` — xem [`crate::tenancy::settings`]). Khách đặt / bỏ qua
//!   `PUT` / `DELETE /api/v1/kernel/settings/{name}`, thử bằng `POST …/settings/{name}/test`
//!   ([`ConnectionProbe`](crate::connections::ConnectionProbe)). Không có biến môi trường nào cho
//!   thứ thuộc về khách.
//! - **Đọc tài khoản lúc chạy** bằng [`ProviderAccounts`] (`context.require::<ProviderAccounts>()`):
//!   trường bắt buộc mà khách chưa đặt → [`not_configured`] (`provider.not_configured`), không bao
//!   giờ rơi về tài khoản của khách khác hay của máy chủ.
//! - **Webhook về**: bộ nối khai [`WebhookEndpoint`]; mỗi khách có một URL riêng kết thúc bằng một
//!   định danh không đoán được ([`ProviderAccounts::webhook_token`]). Module nhận webhook tìm khách
//!   từ định danh ([`ProviderAccounts::resolve_webhook`]) TRƯỚC, rồi bộ nối kiểm chữ ký bằng secret
//!   của đúng khách đó. Khách xem URL của mình ở `GET /api/v1/kernel/connections/webhooks`.
//! - **Gọi ra ngoài** bằng [`http_client`] (không theo chuyển hướng, có hạn giờ; địa chỉ do khách
//!   chọn thì chỉ IP public — D-20261121-ssrf-targets) và [`send`] (log không chứa secret, số đo).
//!   Không gọi trong transaction ghi.
//! - **Chữ ký**: [`hmac_sha256`], [`hex`], [`constant_time_eq`].
//!
//! Hướng dẫn viết một bộ nối: `docs/guides/provider-adapters.md`.

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

/// Mục đích của khoá dẫn xuất cho định danh webhook.
const WEBHOOK_PURPOSE: &str = "kernel.provider_webhook";
/// Số ký tự base64url của phần thô trong định danh webhook (32 ký tự = 192 bit).
const WEBHOOK_TOKEN_LEN: usize = 32;

/// Một đường webhook của một bộ nối: khách dán URL của nó vào trang quản trị của nhà cung cấp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebhookEndpoint {
    connection: &'static str,
    path: &'static str,
    description: &'static str,
}

impl WebhookEndpoint {
    /// Webhook của kết nối `connection` (`<module>.connection.<loại>`) tới `path` — đường dẫn đầy
    /// đủ của route nhận, KHÔNG có định danh cuối (vd `/api/v1/ledger/bank-feeds/sepay`);
    /// `description` (tiếng Anh) nói khách dán URL vào đâu ở nhà cung cấp.
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

/// Câu trả lời nhà cung cấp chờ khi webhook được nhận xong (vd SePay cần `{"success": true}`).
/// Bộ nối trả qua `webhook_ack` của trait; không khai thì module nhận trả envelope chuẩn.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebhookAck {
    /// Mã HTTP (200…299).
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl WebhookAck {
    /// Trả `status` với thân JSON `body`.
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        WebhookAck { status, content_type: "application/json", body: body.into() }
    }

    /// Trả `204 No Content`.
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

/// Tài khoản nhà cung cấp theo khách: đọc cấu hình kết nối của một khách và định tuyến webhook
/// về đúng khách. Kernel công bố; bộ nối lấy bằng `context.require::<ProviderAccounts>()`. Clone rẻ.
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

    /// Ngữ cảnh của khách `id` — trait của bộ nối nhận `TenantId`, cấu hình đọc theo ngữ cảnh.
    pub async fn tenant(&self, id: TenantId) -> Result<TenantContext, Error> {
        self.0.registry.tenant(id).await
    }

    /// Giá trị `setting` của khách; `None` khi khách chưa đặt (trường không bắt buộc).
    pub async fn get(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
    ) -> Result<Option<SettingValue>, Error> {
        self.0.settings.get(tenant, setting).await
    }

    /// Giá trị `setting` của khách; khách chưa đặt → [`not_configured`]. Không bao giờ thay bằng
    /// giá trị của khách khác hay của máy chủ.
    pub async fn require(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
    ) -> Result<SettingValue, Error> {
        self.get(tenant, setting).await?.ok_or_else(|| not_configured(setting.name()))
    }

    /// Khai một đường webhook (gọi trong `Module::start` của bộ nối). Khai lại cùng đường thì bỏ qua.
    pub fn register_webhook(&self, endpoint: WebhookEndpoint) {
        let mut known = self.0.webhooks.write().unwrap_or_else(PoisonError::into_inner);
        if !known.iter().any(|found| found.path == endpoint.path) {
            known.push(endpoint);
        }
    }

    fn endpoints(&self) -> Vec<WebhookEndpoint> {
        self.0.webhooks.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Phần thô của định danh: HMAC-SHA256 của `path` bằng khoá dẫn xuất riêng của khách. Không lưu
    /// ở đâu; cùng khách và cùng đường luôn ra cùng định danh.
    async fn raw_token(&self, tenant: TenantId, path: &str) -> Result<String, Error> {
        let key = self.0.secret.derive_for(tenant, WEBHOOK_PURPOSE).await?;
        let mut encoded = URL_SAFE_NO_PAD.encode(hmac_sha256(&key, path.as_bytes()));
        encoded.truncate(WEBHOOK_TOKEN_LEN);
        Ok(encoded)
    }

    /// Định danh webhook của khách `tenant` cho đường `path` (xem [`WebhookEndpoint::new`]): đoạn
    /// cuối của URL khách dán vào trang của nhà cung cấp. Mang ID khách (trừ khách mặc định) theo
    /// dạng của [`link_token`], nên tìm được khách mà không cần tra bảng nào.
    pub async fn webhook_token(&self, tenant: &TenantContext, path: &str) -> Result<String, Error> {
        Ok(link_token(tenant, &self.raw_token(tenant.id(), path).await?))
    }

    /// URL webhook đầy đủ của khách `tenant` cho đường `path` — cho bộ nối phải gửi URL nhận thông
    /// báo trong từng request (vd `ipnUrl`, `CallbackUrl`). Ghép từ `AUTOXIDE_HTTP_PUBLIC_URL`;
    /// chưa đặt → `public_url_missing` (503). Không bao giờ đoán từ header `Host` của request.
    pub async fn webhook_url(&self, tenant: &TenantContext, path: &str) -> Result<String, Error> {
        let token = self.webhook_token(tenant, path).await?;
        self.0.public_url.link(&format!("{path}/{token}"))
    }

    /// Khách của webhook tới đường `path` với định danh `token`. Định danh sai, khách không có hay
    /// không phục vụ được → [`webhook_unknown`] (cùng một lỗi cho mọi trường hợp). Sau bước này bộ
    /// nối vẫn PHẢI kiểm chữ ký bằng secret của khách tìm được: định danh chỉ để định tuyến.
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

    /// Mọi đường webhook đã khai, kèm URL của khách `tenant`.
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

/// Môi trường của tài khoản nhà cung cấp của một khách (D-20261122-provider-environments): tài
/// khoản thử chỉ chạy với host sandbox của nhà cung cấp. Bộ nối có host sandbox khai cấu hình
/// khách `<module>.connection.<loại>.environment` với `.checked(Environment::check)`, đọc bằng
/// [`Environment::parse`], rồi chọn giữa HAI địa chỉ cố định của người vận hành
/// (`AUTOXIDE_<TÊN>_API_URL`, `AUTOXIDE_<TÊN>_SANDBOX_API_URL`) — khách không bao giờ tự nhập địa chỉ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Environment {
    /// Mặc định khi khách không đặt.
    #[default]
    Production,
    Sandbox,
}

impl Environment {
    /// Giá trị khách đặt: `production` hay `sandbox`; không đặt là `Production`. Giá trị lạ → `None`.
    pub fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim) {
            None | Some("production") => Some(Environment::Production),
            Some("sandbox") => Some(Environment::Sandbox),
            Some(_) => None,
        }
    }

    /// Bộ kiểm cho `TenantSetting::checked`.
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

    /// Chọn địa chỉ theo môi trường.
    pub fn pick<'a>(self, production: &'a str, sandbox: &'a str) -> &'a str {
        match self {
            Environment::Production => production,
            Environment::Sandbox => sandbox,
        }
    }
}

// ---- Lỗi -----------------------------------------------------------------------------------------

/// `provider.not_configured` (422): khách chưa đặt trường `setting` của tài khoản nhà cung cấp.
pub fn not_configured(setting: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "provider.not_configured",
        "This tenant has not set `{setting}`. Set it with `PUT /api/v1/kernel/settings/{setting}`; \
         `GET /api/v1/kernel/settings?group=connection` lists what a connection needs.",
    )
    .with_param("setting", setting)
}

/// `provider.webhook_unknown` (404): định danh trong URL webhook không thuộc khách nào.
pub fn webhook_unknown() -> Error {
    Error::not_found(
        "provider.webhook_unknown",
        "This webhook address is not known. Copy it again from \
         `GET /api/v1/kernel/connections/webhooks`.",
    )
}

// ---- Gọi ra ngoài --------------------------------------------------------------------------------

/// Địa chỉ mà HTTP client của bộ nối gọi tới.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Địa chỉ của nhà cung cấp do bộ nối / người vận hành đặt (không phải khách chọn).
    Fixed,
    /// Địa chỉ do khách chọn (cấu hình của khách): chỉ kết nối tới IP public, trừ khi người vận
    /// hành cho dùng mạng riêng (`allow_private`). Cấu hình đó còn phải `.checked(…)` bằng
    /// [`check_target_url`](crate::net::check_target_url) lúc lưu.
    TenantChosen { allow_private: bool },
}

/// HTTP client cho bộ nối: có hạn giờ, không theo chuyển hướng (gửi secret tới địa chỉ khác địa chỉ
/// đã cấu hình là rủi ro), `User-Agent` của Autoxide.
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

/// Câu trả lời của nhà cung cấp: mã HTTP và thân thô.
#[derive(Debug, Clone)]
pub struct ProviderReply {
    pub status: u16,
    pub body: bytes::Bytes,
}

impl ProviderReply {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Thân đọc thành JSON; `None` khi không phải JSON.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// Không tới được nhà cung cấp (lỗi mạng, hết giờ, thân bị cắt): tạm thời, thử lại được. Bộ nối
/// đổi thành lỗi `…provider_unavailable` của hợp đồng nó cài.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreachable;

/// Gửi `request` tới nhà cung cấp `provider` (tên ổn định của bộ nối) cho việc `purpose` (nhãn cố
/// định, vd `create_payment`). Ghi log (không URL có tham số, không header, không thân — chỉ việc,
/// mã HTTP) và số đo `kernel_provider_calls_total`. Mã ngoài 2xx vẫn là `Ok`: bộ nối tự đọc lỗi
/// theo tài liệu của nhà cung cấp.
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

/// HMAC-SHA256 của `data` bằng `key`.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    // HMAC nhận khoá dài bất kỳ: `new_from_slice` không lỗi được; nhánh lỗi trả chữ ký của khoá rỗng
    // để không cần `unwrap`.
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .unwrap_or_else(|_| <Hmac<Sha256> as KeyInit>::new(&Default::default()));
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// `bytes` dạng hex chữ thường.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// So hai chuỗi byte trong thời gian không phụ thuộc vị trí khác nhau đầu tiên (so chữ ký, API key).
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
