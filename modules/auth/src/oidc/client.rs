//! Gọi nhà cung cấp đăng nhập (bước 40 P3): discovery, JWKS, đổi `code` lấy token, API kiểu
//! GitHub. Quản trị khách khai được issuer, nên mọi lần gọi đi qua bộ lọc SSRF của kernel
//! ([`autoxide_kernel::net`]) trừ khi máy chủ đặt `AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS=true`.
//!
//! Discovery và JWKS giữ trong bộ nhớ theo `Cache-Control: max-age` (kẹp trong 5 phút – 24 giờ).
//! Gặp `kid` lạ thì tải lại JWKS (nhà cung cấp vừa xoay khoá), nhưng mỗi issuer tối đa một lần mỗi
//! phút — kẻ xấu gửi `kid` ngẫu nhiên không biến máy chủ thành công cụ dội request.
//! Không gọi trong transaction ghi (luật dữ liệu): bên gọi đổi `code` xong mới mở transaction.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use autoxide_kernel::error::Error;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

use super::jwt::{Jwk, JwkSet, Unverified};

const MIN_CACHE: Duration = Duration::from_secs(5 * 60);
const MAX_CACHE: Duration = Duration::from_secs(24 * 60 * 60);
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const TIMEOUT: Duration = Duration::from_secs(10);
/// Trần cỡ một response của nhà cung cấp.
const MAX_BODY: usize = 1024 * 1024;

/// Tài liệu `/.well-known/openid-configuration` (OIDC Discovery 1.0 §3), phần cần dùng.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

#[derive(Default)]
struct Cached {
    discovery: Option<(Arc<Discovery>, Instant)>,
    keys: Option<(Arc<JwkSet>, Instant)>,
    /// Lần tải lại JWKS gần nhất vì gặp `kid` lạ (giới hạn một lần mỗi phút).
    refreshed_at: Option<Instant>,
}

/// HTTP client tới nhà cung cấp, kèm cache discovery / JWKS theo issuer. Một bản cho cả tiến trình.
pub(crate) struct OidcClient {
    http: reqwest::Client,
    allow_private_networks: bool,
    cache: Mutex<HashMap<String, Cached>>,
}

impl OidcClient {
    pub(crate) fn new(allow_private_networks: bool) -> Result<Self, Error> {
        autoxide_kernel::tls::install_crypto_provider();
        let mut builder = reqwest::Client::builder()
            .timeout(TIMEOUT)
            // Địa chỉ đã kiểm là địa chỉ được gọi: không theo chuyển hướng tới chỗ khác.
            .redirect(reqwest::redirect::Policy::none())
            // Proxy của hệ thống tự phân giải tên, vượt qua bộ lọc: không dùng.
            .no_proxy()
            .user_agent(concat!("Autoxide/", env!("CARGO_PKG_VERSION")));
        if !allow_private_networks {
            builder = builder.dns_resolver(autoxide_kernel::net::PublicOnly);
        }
        let http = builder
            .build()
            .map_err(|error| Error::internal(format!("OIDC HTTP client: {error}")))?;
        Ok(OidcClient { http, allow_private_networks, cache: Mutex::new(HashMap::new()) })
    }

    /// `raw` là địa chỉ được phép gọi: `https://` (hay `http://` khi cho mạng riêng, để thử), không
    /// user / mật khẩu, host không biết trước là nội bộ.
    pub(crate) fn check_url(&self, raw: &str) -> Result<Url, UrlProblem> {
        let url = Url::parse(raw).map_err(|_| UrlProblem::NotUrl)?;
        if url.host_str().is_none_or(str::is_empty) {
            return Err(UrlProblem::NotUrl);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(UrlProblem::Credentials);
        }
        match url.scheme() {
            "https" => {}
            "http" if self.allow_private_networks => {}
            _ => return Err(UrlProblem::Scheme),
        }
        // reqwest không hỏi bộ phân giải khi host là IP: phải kiểm ở đây.
        if !self.allow_private_networks && !autoxide_kernel::net::is_public_host(&url) {
            return Err(UrlProblem::Private);
        }
        Ok(url)
    }

    /// Gửi `request` tới `url`; trả thân JSON và `max-age` của response.
    async fn send(
        &self,
        url: &str,
        request: impl FnOnce(&reqwest::Client, Url) -> reqwest::RequestBuilder,
    ) -> Result<(Value, Option<Duration>), Error> {
        let parsed = self.check_url(url).map_err(|problem| {
            tracing::warn!(url, why = ?problem, "OIDC provider address refused");
            unavailable()
        })?;
        let mut response = request(&self.http, parsed)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| {
                tracing::warn!(url, %error, "OIDC provider unreachable");
                unavailable()
            })?;
        let status = response.status();
        let max_age = response
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .and_then(max_age);
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            tracing::warn!(url, %error, "OIDC provider answer cut off");
            unavailable()
        })? {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_BODY {
                tracing::warn!(url, "OIDC provider answer too large");
                return Err(unavailable());
            }
        }
        let json: Value = serde_json::from_slice(&body).map_err(|_| {
            tracing::warn!(url, %status, "OIDC provider answered something that is not JSON");
            unavailable()
        })?;
        if !status.is_success() {
            // Nhà cung cấp từ chối `code` (đã dùng, hết hạn, sai `redirect_uri`…): lỗi của lượt
            // đăng nhập, không phải nhà cung cấp hỏng.
            let code = json.get("error").and_then(Value::as_str).unwrap_or_default();
            tracing::info!(url, %status, error = code, "OIDC provider refused the request");
            return Err(super::token_invalid("provider_refused"));
        }
        Ok((json, max_age))
    }

    async fn get(&self, url: &str) -> Result<(Value, Option<Duration>), Error> {
        self.send(url, |client, url| client.get(url)).await
    }

    /// Discovery của `issuer` (cache theo `max-age`).
    pub(crate) async fn discovery(&self, issuer: &str) -> Result<Arc<Discovery>, Error> {
        if let Some(found) = self.cached(issuer, |cached| cached.discovery.clone()) {
            return Ok(found);
        }
        let url = format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));
        let (json, max_age) = self.get(&url).await?;
        let discovery: Discovery = serde_json::from_value(json).map_err(|error| {
            tracing::warn!(issuer, %error, "OIDC discovery document is incomplete");
            unavailable()
        })?;
        let discovery = Arc::new(discovery);
        let until = Instant::now() + clamp(max_age);
        self.update(issuer, |cached| cached.discovery = Some((discovery.clone(), until)));
        Ok(discovery)
    }

    /// Khoá kiểm chữ ký của `token` từ JWKS của `issuer`. `kid` lạ thì tải lại JWKS (tối đa một
    /// lần mỗi phút mỗi issuer); vẫn không có thì `None`.
    pub(crate) async fn key_for(
        &self,
        issuer: &str,
        jwks_uri: &str,
        token: &Unverified<'_>,
    ) -> Result<Option<Jwk>, Error> {
        if let Some(keys) = self.cached(issuer, |cached| cached.keys.clone()) {
            if let Some(key) = token.select(&keys) {
                return Ok(Some(key.clone()));
            }
            // Khoá đang giữ còn hạn mà không có `kid` này: nhà cung cấp vừa xoay khoá — hay kẻ xấu
            // gửi `kid` ngẫu nhiên. Tải lại tối đa một lần mỗi phút.
            let may_refresh = self
                .cached_value(issuer, |cached| cached.refreshed_at)
                .is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL);
            if !may_refresh {
                return Ok(None);
            }
            // Đánh dấu trước khi gọi: nhiều request cùng lúc chỉ một bên tải.
            self.update(issuer, |cached| cached.refreshed_at = Some(Instant::now()));
        }
        let (json, max_age) = self.get(jwks_uri).await?;
        let keys: JwkSet = serde_json::from_value(json).map_err(|error| {
            tracing::warn!(issuer, %error, "OIDC JWKS is not a key set");
            unavailable()
        })?;
        let keys = Arc::new(keys);
        let until = Instant::now() + clamp(max_age);
        self.update(issuer, |cached| cached.keys = Some((keys.clone(), until)));
        Ok(token.select(&keys).cloned())
    }

    /// Đổi `code` lấy token ở `token_endpoint` (RFC 6749 §4.1.3, kèm `code_verifier` của PKCE).
    /// `basic`: gửi `client_id` / `client_secret` bằng HTTP Basic thay vì trong thân.
    pub(crate) async fn exchange(
        &self,
        token_endpoint: &str,
        form: &[(&str, &str)],
        basic: Option<(&str, &str)>,
    ) -> Result<Value, Error> {
        let body = form_body(form);
        let (json, _) = self
            .send(token_endpoint, |client, url| {
                let request = client
                    .post(url)
                    .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(body);
                match basic {
                    // RFC 6749 §2.3.1: hai phần mã hoá form trước khi ghép.
                    Some((id, secret)) => request.basic_auth(encode(id), Some(encode(secret))),
                    None => request,
                }
            })
            .await?;
        Ok(json)
    }

    /// `GET url` với access token của người dùng (API kiểu GitHub).
    pub(crate) async fn get_with_token(&self, url: &str, token: &str) -> Result<Value, Error> {
        let (json, _) = self.send(url, |client, url| client.get(url).bearer_auth(token)).await?;
        Ok(json)
    }

    fn cached<T: Clone>(
        &self,
        issuer: &str,
        read: impl Fn(&Cached) -> Option<(T, Instant)>,
    ) -> Option<T> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let (value, until) = cache.get(issuer).and_then(read)?;
        (until > Instant::now()).then_some(value)
    }

    fn cached_value<T>(&self, issuer: &str, read: impl Fn(&Cached) -> Option<T>) -> Option<T> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.get(issuer).and_then(read)
    }

    fn update(&self, issuer: &str, write: impl FnOnce(&mut Cached)) {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        write(cache.entry(issuer.to_owned()).or_default());
    }
}

/// Vì sao một địa chỉ không được gọi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UrlProblem {
    NotUrl,
    Credentials,
    Scheme,
    Private,
}

/// Thân `application/x-www-form-urlencoded`.
pub(crate) fn form_body(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Mã hoá một giá trị form (WHATWG `application/x-www-form-urlencoded`).
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'*' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn max_age(header: &str) -> Option<Duration> {
    header.split(',').find_map(|part| {
        let seconds = part.trim().strip_prefix("max-age=")?;
        seconds.trim_matches('\u{22}').parse::<u64>().ok().map(Duration::from_secs)
    })
}

fn clamp(max_age: Option<Duration>) -> Duration {
    max_age.unwrap_or(MIN_CACHE).clamp(MIN_CACHE, MAX_CACHE)
}

pub(crate) fn unavailable() -> Error {
    Error::unavailable(
        "auth.oidc_provider_unavailable",
        "The sign-in provider could not be reached. Try again later.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ma_hoa_form_va_cache_control() {
        assert_eq!(form_body(&[("a b", "x&y=z/é")]), "a+b=x%26y%3Dz%2F%C3%A9");
        assert_eq!(max_age("public, max-age=600, must-revalidate"), Some(Duration::from_secs(600)));
        assert_eq!(max_age("no-cache"), None);
        assert_eq!(clamp(Some(Duration::from_secs(1))), MIN_CACHE);
        assert_eq!(clamp(Some(Duration::from_secs(10 * 86_400))), MAX_CACHE);
    }

    #[test]
    fn dia_chi_noi_bo_bi_chan() {
        let strict = OidcClient::new(false).unwrap();
        for url in [
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://169.254.169.254/latest",
            "http://accounts.example.com",
            "https://user:pass@accounts.example.com",
        ] {
            assert!(strict.check_url(url).is_err(), "{url}");
        }
        assert!(strict.check_url("https://accounts.google.com").is_ok());
        let open = OidcClient::new(true).unwrap();
        assert!(open.check_url("http://127.0.0.1:8080").is_ok());
    }
}
