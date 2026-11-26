//! Nhận diện client OAuth (D-20261126-mcp-oauth mục 4):
//!
//! - **Client ID Metadata Document** (đường chính, draft-ietf-oauth-client-id-metadata-document-03): `client_id`
//!   là URL `https` có path; máy chủ tải tài liệu JSON ở chính URL đó qua bộ lọc SSRF của kernel
//!   (`net::PublicOnly`, không proxy, không theo chuyển hướng, chỉ 200, tối đa 5 KB), kiểm `client_id` trùng URL,
//!   cấm secret, chỉ nhận client public (`none`). Giữ trong bộ nhớ theo `Cache-Control` (kẹp 5 phút – 24 giờ); lỗi
//!   giữ 30 giây để không bị dùng làm công cụ dội request. Không tải `logo_uri`; `client_name` là dữ liệu chưa tin.
//! - **Đăng ký động** (RFC 7591, deprecated trong MCP 2026-07-28): `POST /oauth/register` lưu client public ở khách
//!   nền tảng, `client_id` = `axdc_<id>`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt as _, TenantContext};
use autoxide_kernel::time::Timestamp;
use axum::http::StatusCode;
use reqwest::Url;
use sea_orm::{ActiveModelTrait as _, ColumnTrait as _, EntityTrait as _, QueryFilter as _, Set};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::OAuthError;
use super::entity::client as entity;

/// Tiền tố `client_id` của client đăng ký động.
pub(crate) const DYNAMIC_PREFIX: &str = "axdc_";
const MAX_DOCUMENT: usize = 5 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MIN_CACHE: Duration = Duration::from_secs(5 * 60);
const MAX_CACHE: Duration = Duration::from_secs(24 * 60 * 60);
const FAILURE_CACHE: Duration = Duration::from_secs(30);
/// Số tài liệu giữ trong bộ nhớ; đầy thì bỏ các mục đã hết hạn rồi mục cũ nhất.
const MAX_CACHED: usize = 1000;
/// Số lần tải đồng thời: `/authorize` chưa xác thực mà gọi ra ngoài, nên có trần.
const MAX_FETCHES: usize = 8;
const NAME_MAX: usize = 200;
/// Số lần tải tài liệu từ một host mỗi phút.
const FETCHES_PER_HOST: u32 = 20;
const HOST_WINDOW: Duration = Duration::from_secs(60);
const MAX_REDIRECTS: usize = 10;

/// Một client đã nhận diện.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisteredClient {
    pub client_id: String,
    /// Tên client tự khai (chưa tin: chỉ để hiển thị cùng host của `client_id`).
    pub name: String,
    pub redirect_uris: Vec<String>,
    /// `cimd` hay `dynamic`.
    pub registration: &'static str,
}

impl RegisteredClient {
    /// `redirect_uri` có nằm trong danh sách: so khớp chính xác, trừ cổng của địa chỉ loopback (RFC 8252 §7.3,
    /// RFC 9700 §4.1.1).
    pub(crate) fn allows_redirect(&self, redirect_uri: &str) -> bool {
        self.redirect_uris.iter().any(|registered| {
            registered == redirect_uri
                || matches!(
                    (without_loopback_port(registered), without_loopback_port(redirect_uri)),
                    (Some(left), Some(right)) if left == right
                )
        })
    }
}

/// `http://127.0.0.1:1234/cb` → `http://127.0.0.1/cb` (chỉ với địa chỉ loopback qua `http`).
fn without_loopback_port(uri: &str) -> Option<String> {
    let url = Url::parse(uri).ok()?;
    // Chỉ địa chỉ IP loopback được linh hoạt cổng; tên `localhost` thì không (RFC 8252 §7.3, §8.3).
    let literal = url.host_str().is_some_and(|host| host.eq_ignore_ascii_case("localhost"));
    if url.scheme() != "http" || literal || !is_loopback(&url) {
        return None;
    }
    let mut url = url;
    url.set_port(None).ok()?;
    Some(url.to_string())
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else { return false };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

/// Lỗi kiểm một `redirect_uri` client khai: `https`, hay `http` tới loopback; không fragment.
pub(crate) fn redirect_problem(uri: &str) -> Option<&'static str> {
    let Ok(url) = Url::parse(uri) else { return Some("is not an absolute URL") };
    if url.fragment().is_some() {
        return Some("must not have a fragment");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Some("must not carry user information");
    }
    match url.scheme() {
        "https" => None,
        "http" if is_loopback(&url) => None,
        _ => Some("must use https, or http to a loopback address"),
    }
}

/// Mọi redirect đều trỏ về máy người dùng (`localhost`): trang consent cảnh báo (CIMD -03 §10.2).
pub(crate) fn localhost_only(redirects: &[String]) -> bool {
    !redirects.is_empty()
        && redirects.iter().all(|uri| Url::parse(uri).is_ok_and(|url| is_loopback(&url)))
}

/// Host của một URL (hiện ở trang consent).
pub(crate) fn host_of(uri: &str) -> String {
    Url::parse(uri).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default()
}

/// Tài liệu metadata của client, phần cần dùng.
#[derive(Debug, Deserialize)]
struct Document {
    client_id: Option<String>,
    client_name: Option<String>,
    #[serde(default)]
    redirect_uris: Vec<String>,
    #[serde(default)]
    grant_types: Option<Vec<String>>,
    #[serde(default)]
    response_types: Option<Vec<String>>,
    token_endpoint_auth_method: Option<String>,
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
    client_secret: Option<Value>,
    client_secret_expires_at: Option<Value>,
}

enum Cached {
    Client(RegisteredClient),
    Failed(String),
}

/// Bộ nhận diện client: HTTP client chống SSRF và cache tài liệu metadata. Một bản cho cả tiến trình.
pub(crate) struct ClientDirectory {
    http: reqwest::Client,
    allow_private_networks: bool,
    issuer: String,
    cache: Mutex<HashMap<String, (Cached, Instant)>>,
    fetches: Semaphore,
    /// Số lần tải theo host trong phút hiện tại: một host không bị dùng làm đích dội request.
    hosts: Mutex<HashMap<String, (u32, Instant)>>,
}

impl ClientDirectory {
    pub(crate) fn new(allow_private_networks: bool, issuer: String) -> Result<Self, Error> {
        autoxide_kernel::tls::install_crypto_provider();
        let mut builder = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            // Địa chỉ đã kiểm là địa chỉ được gọi: không theo chuyển hướng (CIMD -03 §6).
            .redirect(reqwest::redirect::Policy::none())
            // Proxy của hệ thống tự phân giải tên, vượt qua bộ lọc: không dùng.
            .no_proxy()
            .user_agent(concat!("Autoxide/", env!("CARGO_PKG_VERSION")));
        if !allow_private_networks {
            builder = builder.dns_resolver(autoxide_kernel::net::PublicOnly);
        }
        let http = builder
            .build()
            .map_err(|error| Error::internal(format!("OAuth client HTTP client: {error}")))?;
        Ok(ClientDirectory {
            http,
            allow_private_networks,
            issuer,
            cache: Mutex::new(HashMap::new()),
            fetches: Semaphore::new(MAX_FETCHES),
            hosts: Mutex::new(HashMap::new()),
        })
    }

    /// Nhận diện `client_id`. Lỗi trả mô tả cho người (trang lỗi trước khi tin `redirect_uri`).
    pub(crate) async fn resolve(
        &self,
        platform: &TenantContext,
        client_id: &str,
    ) -> Result<RegisteredClient, OAuthError> {
        if client_id.starts_with("https://") || client_id.starts_with("http://") {
            return self.metadata_document(client_id).await;
        }
        if client_id.starts_with(DYNAMIC_PREFIX) {
            let row = entity::Entity::find()
                .filter(entity::Column::ClientId.eq(client_id))
                .scoped(platform)
                .one(platform.database().read())
                .await?;
            if let Some(row) = row {
                touch(platform, &row).await;
                let redirect_uris: Vec<String> =
                    serde_json::from_str(&row.redirect_uris).unwrap_or_default();
                return Ok(RegisteredClient {
                    client_id: row.client_id,
                    name: row.client_name,
                    redirect_uris,
                    registration: "dynamic",
                });
            }
        }
        Err(OAuthError::invalid_client(
            "Unknown client_id: use the URL of your client metadata document, or register first.",
        ))
    }

    async fn metadata_document(&self, client_id: &str) -> Result<RegisteredClient, OAuthError> {
        let url = self.check_client_id(client_id)?;
        if let Some(cached) = self.cached(client_id) {
            return cached;
        }
        if !self.host_allows(url.host_str().unwrap_or_default()) {
            return Err(OAuthError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily_unavailable",
                "Too many client metadata fetches for this host; retry in a minute.",
            ));
        }
        let Ok(_permit) = self.fetches.try_acquire() else {
            return Err(OAuthError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily_unavailable",
                "Too many client metadata documents are being fetched; retry in a moment.",
            ));
        };
        let outcome = self.fetch(client_id, url).await;
        let (entry, ttl) = match &outcome {
            Ok((client, ttl)) => (Cached::Client(client.clone()), *ttl),
            Err(description) => (Cached::Failed(description.clone()), FAILURE_CACHE),
        };
        self.remember(client_id, entry, ttl);
        outcome.map(|(client, _)| client).map_err(OAuthError::invalid_client)
    }

    /// Còn được tải từ `host` trong phút này không (đếm luôn lần này).
    fn host_allows(&self, host: &str) -> bool {
        let Ok(mut hosts) = self.hosts.lock() else { return false };
        let now = Instant::now();
        if hosts.len() >= MAX_CACHED {
            hosts.retain(|_, (_, since)| now.duration_since(*since) < HOST_WINDOW);
        }
        let entry = hosts.entry(host.to_ascii_lowercase()).or_insert((0, now));
        if now.duration_since(entry.1) >= HOST_WINDOW {
            *entry = (0, now);
        }
        entry.0 += 1;
        entry.0 <= FETCHES_PER_HOST
    }

    fn cached(&self, client_id: &str) -> Option<Result<RegisteredClient, OAuthError>> {
        let cache = self.cache.lock().ok()?;
        let (entry, until) = cache.get(client_id)?;
        if *until <= Instant::now() {
            return None;
        }
        Some(match entry {
            Cached::Client(client) => Ok(client.clone()),
            Cached::Failed(description) => Err(OAuthError::invalid_client(description.clone())),
        })
    }

    fn remember(&self, client_id: &str, entry: Cached, ttl: Duration) {
        let Ok(mut cache) = self.cache.lock() else { return };
        if cache.len() >= MAX_CACHED {
            let now = Instant::now();
            cache.retain(|_, (_, until)| *until > now);
            if cache.len() >= MAX_CACHED
                && let Some(oldest) =
                    cache.iter().min_by_key(|(_, (_, until))| *until).map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(client_id.to_owned(), (entry, Instant::now() + ttl));
    }

    /// `client_id` đúng dạng CIMD -03 §3: `https`, có path khác `/`, không user, fragment, query hay đoạn `.`/`..`;
    /// không trùng issuer.
    fn check_client_id(&self, client_id: &str) -> Result<Url, OAuthError> {
        let invalid = |why: &str| OAuthError::invalid_client(format!("The client_id URL {why}."));
        if client_id.len() > super::consent::MAX_URI {
            return Err(invalid("is longer than 2048 characters"));
        }
        let url = Url::parse(client_id).map_err(|_| invalid("is not a valid URL"))?;
        // So `client_id` bằng chuỗi (CIMD -03 §3): scheme và host viết thường, để một client không có hai khoá cache.
        let authority = client_id
            .split_once("://")
            .map_or("", |(_, rest)| rest.split('/').next().unwrap_or_default());
        if client_id
            .split_once("://")
            .is_some_and(|(scheme, _)| scheme != scheme.to_ascii_lowercase())
            || authority != authority.to_ascii_lowercase()
        {
            return Err(invalid("must use a lowercase scheme and host"));
        }
        // Chỉ cổng 443 (mặc định của https): không quét cổng khác của máy chủ ngoài.
        if !self.allow_private_networks && url.port().is_some_and(|port| port != 443) {
            return Err(invalid("must use the default https port"));
        }
        match url.scheme() {
            "https" => {}
            "http" if self.allow_private_networks => {}
            _ => return Err(invalid("must use https")),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(invalid("must not carry user information"));
        }
        if url.fragment().is_some() || url.query().is_some() {
            return Err(invalid("must not have a query or a fragment"));
        }
        let path =
            client_id.split_once("://").and_then(|(_, rest)| rest.find('/').map(|at| &rest[at..]));
        let Some(path) = path.filter(|path| *path != "/") else {
            return Err(invalid("must have a path"));
        };
        if path.split('/').any(|segment| segment == "." || segment == "..") {
            return Err(invalid("must not contain `.` or `..` segments"));
        }
        if client_id.trim_end_matches('/') == self.issuer {
            return Err(invalid("is this server"));
        }
        // reqwest không hỏi bộ phân giải khi host là IP: kiểm ở đây.
        if !self.allow_private_networks && !autoxide_kernel::net::is_public_host(&url) {
            return Err(invalid("points to a private address"));
        }
        Ok(url)
    }

    async fn fetch(
        &self,
        client_id: &str,
        url: Url,
    ) -> Result<(RegisteredClient, Duration), String> {
        let mut response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| {
                tracing::info!(client_id, %error, "OAuth client metadata unreachable");
                "The client metadata document could not be fetched.".to_owned()
            })?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!(
                "The client metadata document answered {} instead of 200.",
                response.status().as_u16()
            ));
        }
        let ttl = response
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .and_then(max_age)
            .unwrap_or(MIN_CACHE)
            .clamp(MIN_CACHE, MAX_CACHE);
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The client metadata document was cut off.".to_owned())?
        {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_DOCUMENT {
                return Err("The client metadata document is larger than 5 KB.".to_owned());
            }
        }
        let document: Document = serde_json::from_slice(&body)
            .map_err(|_| "The client metadata document is not valid JSON metadata.".to_owned())?;
        Ok((validate(client_id, document)?, ttl))
    }
}

/// Ghi lần dùng của client đăng ký động, nhiều nhất mỗi ngày một lần (job dọn client bỏ rơi đọc nó). Hỏng thì chỉ ghi
/// log.
async fn touch(platform: &TenantContext, row: &entity::Model) {
    let now = Timestamp::now();
    if row.last_used_at.is_some_and(|last| last + jiff::SignedDuration::from_hours(24) > now) {
        return;
    }
    let result = entity::Entity::update_many()
        .col_expr(entity::Column::LastUsedAt, sea_orm::sea_query::Expr::value(Some(now)))
        .filter(entity::Column::TenantId.eq(platform.id().as_uuid()))
        .filter(entity::Column::Id.eq(row.id))
        .exec(platform.database().write())
        .await;
    if let Err(error) = result {
        tracing::warn!(client_id = row.client_id, %error, "could not record the use of an OAuth client");
    }
}

/// Kiểm tài liệu (CIMD -03 §4, §4.1, §5).
fn validate(client_id: &str, document: Document) -> Result<RegisteredClient, String> {
    if document.client_id.as_deref() != Some(client_id) {
        return Err("The client_id in the metadata document must equal its URL exactly.".to_owned());
    }
    if document.client_secret.is_some() || document.client_secret_expires_at.is_some() {
        return Err("A client metadata document must not contain a client secret.".to_owned());
    }
    let methods = document
        .token_endpoint_auth_methods_supported
        .or_else(|| document.token_endpoint_auth_method.map(|method| vec![method]));
    if methods.is_some_and(|methods| !methods.iter().any(|method| method == "none")) {
        return Err("This server only accepts public clients (token_endpoint_auth_method `none`)."
            .to_owned());
    }
    if document
        .grant_types
        .is_some_and(|types| !types.iter().any(|kind| kind == "authorization_code"))
    {
        return Err("The client must use the `authorization_code` grant.".to_owned());
    }
    if document.response_types.is_some_and(|types| !types.iter().any(|kind| kind == "code")) {
        return Err("The client must use the `code` response type.".to_owned());
    }
    let redirect_uris = checked_redirects(document.redirect_uris)?;
    let name = document.client_name.map(|name| clip(&name)).filter(|name| !name.is_empty());
    let Some(name) = name else {
        return Err("The client metadata document must have a client_name.".to_owned());
    };
    Ok(RegisteredClient {
        client_id: client_id.to_owned(),
        name,
        redirect_uris,
        registration: "cimd",
    })
}

fn checked_redirects(redirect_uris: Vec<String>) -> Result<Vec<String>, String> {
    if redirect_uris.is_empty() || redirect_uris.len() > MAX_REDIRECTS {
        return Err(format!("The client must list 1 to {MAX_REDIRECTS} redirect_uris."));
    }
    for uri in &redirect_uris {
        if uri.len() > super::consent::MAX_URI {
            return Err("every redirect_uri must be at most 2048 characters".to_owned());
        }
        if let Some(problem) = redirect_problem(uri) {
            return Err(format!("redirect_uri `{uri}` {problem}."));
        }
    }
    Ok(redirect_uris)
}

/// Tên hiển thị: bỏ ký tự điều khiển, tối đa 200 ký tự.
fn clip(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control() && !is_invisible(*character))
        .take(NAME_MAX)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Ký tự định dạng vô hình (đảo hướng chữ, khoảng trắng độ rộng 0): giả mạo tên hiển thị được.
fn is_invisible(character: char) -> bool {
    matches!(character, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

fn max_age(value: &str) -> Option<Duration> {
    if value.split(',').any(|part| matches!(part.trim(), "no-store" | "no-cache")) {
        return Some(Duration::ZERO);
    }
    value.split(',').find_map(|part| {
        part.trim()
            .strip_prefix("max-age=")
            .and_then(|seconds| seconds.parse().ok())
            .map(Duration::from_secs)
    })
}

/// Dynamic client registration request (RFC 7591 §2). Unknown fields are ignored (RFC 7591 §3.1).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = OAuthClientRegistration)]
pub(crate) struct Registration {
    /// Where the browser comes back with the code: `https` URLs, or `http` to a loopback address.
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    /// Name shown to the person approving the client.
    pub client_name: Option<String>,
    /// Only `none` (public client using PKCE).
    pub token_endpoint_auth_method: Option<String>,
    /// `authorization_code` and optionally `refresh_token`.
    pub grant_types: Option<Vec<String>>,
    /// Only `code`.
    pub response_types: Option<Vec<String>>,
}

/// Answer of dynamic registration (RFC 7591 §3.2.1).
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
#[schema(as = OAuthClientRegistered)]
pub(crate) struct RegistrationResponse {
    /// Use it as `client_id` in the authorization and token requests.
    pub client_id: String,
    /// When it was issued (Unix seconds).
    pub client_id_issued_at: i64,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    /// Always `none`: a public client proving itself with PKCE.
    pub token_endpoint_auth_method: &'static str,
    pub grant_types: Vec<&'static str>,
    pub response_types: Vec<&'static str>,
}

impl From<RegisteredClient> for RegistrationResponse {
    fn from(client: RegisteredClient) -> Self {
        RegistrationResponse {
            client_id: client.client_id,
            client_id_issued_at: Timestamp::now().as_unix_seconds(),
            client_name: client.name,
            redirect_uris: client.redirect_uris,
            token_endpoint_auth_method: "none",
            grant_types: vec!["authorization_code", "refresh_token"],
            response_types: vec!["code"],
        }
    }
}

/// Tạo client đăng ký động ở khách nền tảng.
pub(crate) async fn register(
    platform: &TenantContext,
    input: Registration,
) -> Result<RegisteredClient, OAuthError> {
    let metadata = |description: String| {
        OAuthError::new(StatusCode::BAD_REQUEST, "invalid_client_metadata", description)
    };
    if input.token_endpoint_auth_method.as_deref().is_some_and(|method| method != "none") {
        return Err(metadata(
            "Only public clients are accepted: token_endpoint_auth_method `none`.".to_owned(),
        ));
    }
    if input.grant_types.as_ref().is_some_and(|types| {
        types.iter().any(|kind| kind != "authorization_code" && kind != "refresh_token")
    }) {
        return Err(metadata(
            "grant_types can only be `authorization_code` and `refresh_token`.".to_owned(),
        ));
    }
    if input.response_types.as_ref().is_some_and(|types| types.iter().any(|kind| kind != "code")) {
        return Err(metadata("response_types can only be `code`.".to_owned()));
    }
    let redirect_uris = checked_redirects(input.redirect_uris).map_err(|description| {
        OAuthError::new(StatusCode::BAD_REQUEST, "invalid_redirect_uri", description)
    })?;
    let name = input.client_name.map(|name| clip(&name)).filter(|name| !name.is_empty());
    let name = name.unwrap_or_else(|| "Unnamed MCP client".to_owned());
    let id = Uuid::now_v7();
    let client_id = format!("{DYNAMIC_PREFIX}{}", id.simple());
    entity::ActiveModel {
        tenant_id: Set(platform.id().as_uuid()),
        id: Set(id),
        client_id: Set(client_id.clone()),
        client_name: Set(name.clone()),
        redirect_uris: Set(serde_json::to_string(&redirect_uris).map_err(Error::internal)?),
        created_at: Set(Timestamp::now()),
        last_used_at: Set(None),
    }
    .insert(platform.database().write())
    .await?;
    tracing::info!(client_id, "OAuth client registered");
    Ok(RegisteredClient { client_id, name, redirect_uris, registration: "dynamic" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(value: Value) -> Document {
        serde_json::from_value(value).unwrap()
    }

    const ID: &str = "https://client.example/oauth/client.json";

    #[test]
    fn tai_lieu_hop_le_va_khong_hop_le() {
        let good = json_doc(serde_json::json!({}));
        assert_eq!(validate(ID, good).unwrap().name, "Example");
        let wrong_id = json_doc(serde_json::json!({"client_id": "https://other.example/c.json"}));
        assert!(validate(ID, wrong_id).is_err());
        let secret = json_doc(serde_json::json!({"client_secret": "x"}));
        assert!(validate(ID, secret).is_err());
        let confidential =
            json_doc(serde_json::json!({"token_endpoint_auth_method": "client_secret_basic"}));
        assert!(validate(ID, confidential).is_err());
        let both = json_doc(
            serde_json::json!({"token_endpoint_auth_methods_supported": ["private_key_jwt", "none"]}),
        );
        assert!(validate(ID, both).is_ok());
        let no_redirect = json_doc(serde_json::json!({"redirect_uris": []}));
        assert!(validate(ID, no_redirect).is_err());
        let plain_http = json_doc(serde_json::json!({"redirect_uris": ["http://evil.example/cb"]}));
        assert!(validate(ID, plain_http).is_err());
        let control = json_doc(serde_json::json!({"client_name": "A\u{202e}B"}));
        assert_eq!(validate(ID, control).unwrap().name, "AB");
    }

    fn json_doc(overrides: Value) -> Document {
        let mut base = serde_json::json!({
            "client_id": ID,
            "client_name": "Example",
            "redirect_uris": ["https://client.example/cb", "http://127.0.0.1:33418/cb"],
        });
        if let (Value::Object(base), Value::Object(overrides)) = (&mut base, overrides) {
            base.extend(overrides);
        }
        document(base)
    }

    #[test]
    fn redirect_loopback_bo_qua_cong() {
        let client = RegisteredClient {
            client_id: ID.to_owned(),
            name: "x".to_owned(),
            redirect_uris: vec![
                "http://127.0.0.1/cb".to_owned(),
                "https://c.example/cb".to_owned(),
            ],
            registration: "cimd",
        };
        assert!(client.allows_redirect("http://127.0.0.1:51234/cb"));
        assert!(client.allows_redirect("https://c.example/cb"));
        assert!(!client.allows_redirect("https://c.example:8443/cb"));
        assert!(!client.allows_redirect("https://c.example/cb/other"));
        assert!(!client.allows_redirect("http://127.0.0.1:1/other"));
        // `localhost` không được linh hoạt cổng (RFC 8252 §8.3).
        let named = RegisteredClient {
            redirect_uris: vec!["http://localhost/cb".to_owned()],
            ..client.clone()
        };
        assert!(!named.allows_redirect("http://localhost:5555/cb"));
        assert!(named.allows_redirect("http://localhost/cb"));
        assert!(localhost_only(&["http://localhost:1/cb".to_owned()]));
        assert!(!localhost_only(&client.redirect_uris));
    }

    #[test]
    fn client_id_dung_dang() {
        let directory = ClientDirectory::new(false, "https://api.example.com".to_owned()).unwrap();
        assert!(directory.check_client_id(ID).is_ok());
        for bad in [
            "http://client.example/c.json",
            "https://client.example",
            "https://client.example/",
            "https://u@client.example/c.json",
            "https://client.example/c.json#x",
            "https://client.example/c.json?x=1",
            "https://client.example/a/../c.json",
            "https://127.0.0.1/c.json",
            "https://10.0.0.1/c.json",
            "HTTPS://client.example/c.json",
            "https://Client.example/c.json",
            "https://client.example:8443/c.json",
        ] {
            assert!(directory.check_client_id(bad).is_err(), "{bad}");
        }
        assert_eq!(max_age("public, max-age=600"), Some(Duration::from_secs(600)));
        assert_eq!(max_age("no-store"), Some(Duration::ZERO));
    }
}
