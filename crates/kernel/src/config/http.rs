//! HTTP server configuration.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum::http::HeaderName;

use super::{ConfigReader, Invalid, Setting, parse_in_range};

pub const ADDRESS: Setting = Setting::new(
    "AUTOXIDE_HTTP_ADDRESS",
    "Address and port the HTTP server listens on. Use `0.0.0.0:8080` to accept connections \
     from other machines (for example inside Docker).",
)
.with_default("127.0.0.1:8080");

pub const REQUEST_TIMEOUT_SECONDS: Setting = Setting::new(
    "AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS",
    "Maximum time to handle one request. A request that takes longer is answered with 503.",
)
.with_default("30");

pub const BODY_LIMIT_BYTES: Setting = Setting::new(
    "AUTOXIDE_HTTP_BODY_LIMIT_BYTES",
    "Maximum size of a request body, in bytes. Larger requests are answered with 413.",
)
.with_default("1048576");

pub const CORS_ORIGINS: Setting = Setting::new(
    "AUTOXIDE_HTTP_CORS_ORIGINS",
    "Web origins allowed to call the API from a browser, separated by commas, or `*` for any \
     origin. When unset, browsers on other origins are refused. In multi-tenant mode a page at \
     `https://<domain>` of a tenant's verified domain is also allowed, without listing it here, \
     but only for requests to that tenant; origins listed here may call for any tenant.",
)
.with_example("https://app.example.com,https://admin.example.com");

pub const SHUTDOWN_SECONDS: Setting = Setting::new(
    "AUTOXIDE_HTTP_SHUTDOWN_SECONDS",
    "On shutdown, how long to wait for requests in progress to finish before closing them.",
)
.with_default("30");

pub const DRAIN_SECONDS: Setting = Setting::new(
    "AUTOXIDE_HTTP_DRAIN_SECONDS",
    "On shutdown, how long to keep handling requests while `/health/ready` answers 503, before \
     the server stops accepting connections. Load balancers then stop sending new requests in \
     time, so a rolling deploy drops none. Set it longer than your load balancer needs to mark \
     an instance down (health check interval times failure threshold, for example `15`). `0` \
     stops accepting connections at once, for a single instance with no load balancer. \
     AUTOXIDE_HTTP_SHUTDOWN_SECONDS starts counting after this.",
)
.with_default("0");

pub const MAX_CONNECTIONS: Setting = Setting::new(
    "AUTOXIDE_HTTP_MAX_CONNECTIONS",
    "Most connections the API port keeps open at once, WebSockets included. At the limit the server \
     stops accepting until one closes: new connections wait in the operating system's queue instead \
     of being refused, so a flood of idle connections cannot use up the process's file descriptors. \
     Keep it below the open file limit (`ulimit -n`) with room for database connections and files; \
     the server warns at startup when it is not. `0`: no limit (a reverse proxy in front limits \
     connections per client better; this is the last line).",
)
.with_default("4096");

pub const API_DOCS: Setting = Setting::new(
    "AUTOXIDE_HTTP_API_DOCS",
    "Serve an interactive API reference at `/api/docs` (`true` or `false`). The page loads its \
     viewer script from a public CDN. The OpenAPI document itself is always served at \
     `/api/v1/openapi.json`.",
)
.with_default("false");

pub const COMPRESSION: Setting = Setting::new(
    "AUTOXIDE_HTTP_COMPRESSION",
    "Compress JSON and text responses with gzip or brotli when the client accepts it (`true` or \
     `false`). Files such as images are never compressed again. Turn it off only when a reverse \
     proxy or CDN in front of Autoxide already compresses responses.",
)
.with_default("true");

pub const PUBLIC_CACHE_SECONDS: Setting = Setting::new(
    "AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS",
    "How long a shared cache (CDN, reverse proxy) may keep a public read of a website (posts, \
     pages, categories, sitemap, feed) before asking again, in seconds (0 to 86400). Browsers always \
     ask again; with `0` shared caches do too. Signed-in requests are never cached by shared \
     caches.",
)
.with_default("60");

pub const METRICS: Setting = Setting::new(
    "AUTOXIDE_HTTP_METRICS",
    "Serve Prometheus metrics at `/metrics` (`true` or `false`). The endpoint has no \
     authentication: set AUTOXIDE_HTTP_METRICS_ADDRESS to serve it on its own port that only \
     your metrics collector reaches, or keep the API port off the internet.",
)
.with_default("false");

pub const METRICS_ADDRESS: Setting = Setting::new(
    "AUTOXIDE_HTTP_METRICS_ADDRESS",
    "With AUTOXIDE_HTTP_METRICS=true: address and port of a second listener that serves only \
     `/metrics`, which then leaves the API port. Publish this port only to your metrics collector \
     (for example `0.0.0.0:9090` inside Docker or Kubernetes, without publishing it to the \
     internet). When unset, `/metrics` is served on AUTOXIDE_HTTP_ADDRESS.",
)
.with_example("127.0.0.1:9090");

pub const RATE_LIMIT_PER_MINUTE: Setting = Setting::new(
    "AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE",
    "Requests one client address may make per minute, on average; more are answered with 429 \
     and a `Retry-After` header. `0` turns rate limiting off. Health checks are not counted. \
     Counted per server process.",
)
.with_default("600");

pub const RATE_LIMIT_BURST: Setting = Setting::new(
    "AUTOXIDE_HTTP_RATE_LIMIT_BURST",
    "Requests one client address may make at once before the per-minute rate applies.",
)
.with_default("60");

pub const API_KEY_RATE_LIMIT_PER_MINUTE: Setting = Setting::new(
    "AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE",
    "Requests one API key may make per minute, on average (REST and MCP), on top of the limit \
     per client address; more are answered with 429 and a `Retry-After` header. AI agents often \
     retry in loops: this keeps one key from crowding out the others. `0` turns it off. \
     Counted per server process, or shared by every server with \
     `AUTOXIDE_HTTP_RATE_LIMIT_STORE=database`.",
)
.with_default("120");

pub const API_KEY_RATE_LIMIT_BURST: Setting = Setting::new(
    "AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_BURST",
    "Requests one API key may make at once before the per-minute rate applies.",
)
.with_default("30");

pub const RATE_LIMIT_STORE: Setting = Setting::new(
    "AUTOXIDE_HTTP_RATE_LIMIT_STORE",
    "Where the limit per API key is counted: `memory` (each server process counts on its own; \
     exact with SQLite, which runs one server) or `database` (shared by every server through the \
     database, for several servers on PostgreSQL). `database` counts in fixed one-minute windows \
     of at most max(per minute, burst) requests and takes them in lots of max(1, burst / 4), so \
     with N servers a key may get up to N lots more per minute. The limit per client address is \
     always counted in memory.",
)
.with_default("memory");

pub const CLIENT_IP_HEADER: Setting = Setting::new(
    "AUTOXIDE_HTTP_CLIENT_IP_HEADER",
    "Behind a reverse proxy, the header in which the proxy passes the client's address, \
     usually `X-Forwarded-For`. Set it only when every request goes through your proxies: \
     clients can forge the header otherwise. When unset, the address of the connection is \
     used.",
)
.with_example("X-Forwarded-For");

pub const TRUSTED_PROXY_COUNT: Setting = Setting::new(
    "AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT",
    "With AUTOXIDE_HTTP_CLIENT_IP_HEADER: how many of your own proxies a request passes \
     before the API (1 to 10). Each adds an address at the end of the header; the client's is \
     this many from the end. `1` for one reverse proxy; `2` for a CDN in front of a website \
     server (such as Next.js) that calls the API and forwards the header. A wrong count makes \
     every visitor look like one client, or lets clients choose their address.",
)
.with_default("1");

pub const TRUSTED_PROXIES: Setting = Setting::new(
    "AUTOXIDE_HTTP_TRUSTED_PROXIES",
    "With AUTOXIDE_HTTP_CLIENT_IP_HEADER: the addresses of your own proxies, as a \
     comma-separated list of IP addresses and ranges (`10.0.0.0/8, 2001:db8::/32, \
     192.0.2.7`). When set, it replaces AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT: the header is read \
     only when the connection comes from one of these addresses, and the client is the last \
     address in the header that is not one of them. Safer than a count when clients could \
     reach one of your proxies directly, or take different paths to the API.",
)
.with_example("10.0.0.0/8, 192.0.2.7");

pub const SECURITY_HEADERS: Setting = Setting::new(
    "AUTOXIDE_HTTP_SECURITY_HEADERS",
    "Add security headers to every response (`true` or `false`): `X-Content-Type-Options`, \
     `Referrer-Policy`, `X-Frame-Options`, a strict `Content-Security-Policy`, and \
     `Strict-Transport-Security` when AUTOXIDE_HTTP_HSTS_SECONDS is set. A header a handler sets \
     itself is kept. Keep it on even when a reverse proxy adds its own headers.",
)
.with_default("true");

pub const REFERRER_POLICY: Setting = Setting::new(
    "AUTOXIDE_HTTP_REFERRER_POLICY",
    "Value of the `Referrer-Policy` header: one of `no-referrer`, `no-referrer-when-downgrade`, \
     `origin`, `origin-when-cross-origin`, `same-origin`, `strict-origin`, \
     `strict-origin-when-cross-origin`, `unsafe-url`.",
)
.with_default("no-referrer");

pub const HSTS_SECONDS: Setting = Setting::new(
    "AUTOXIDE_HTTP_HSTS_SECONDS",
    "`max-age` of the `Strict-Transport-Security` header, in seconds (0 to 63072000). Set it \
     only when the API is served over HTTPS (`31536000` is one year): browsers then refuse plain \
     HTTP to this host for that long. `0` sends no such header.",
)
.with_default("0");

pub const REQUEST_LOG_DAYS: Setting = Setting::new(
    "AUTOXIDE_REQUEST_LOG_DAYS",
    "Days the per-tenant log of signed-in API and MCP calls is kept (0 to 365); `0` turns the \
     log off. No bodies, tokens or query strings are logged.",
)
.with_default("14");

pub const HSTS_INCLUDE_SUBDOMAINS: Setting = Setting::new(
    "AUTOXIDE_HTTP_HSTS_INCLUDE_SUBDOMAINS",
    "Add `includeSubDomains` to the `Strict-Transport-Security` header (`true` or `false`): \
     every subdomain must then be served over HTTPS too.",
)
.with_default("false");

/// Standard `Referrer-Policy` values (W3C Referrer Policy).
const REFERRER_POLICIES: [&str; 8] = [
    "no-referrer",
    "no-referrer-when-downgrade",
    "origin",
    "origin-when-cross-origin",
    "same-origin",
    "strict-origin",
    "strict-origin-when-cross-origin",
    "unsafe-url",
];

/// Security headers attached to every response (step 40 P1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SecurityHeaders {
    /// `false`: no header is attached.
    pub enabled: bool,
    pub referrer_policy: String,
    /// HSTS `max-age`; 0 = not sent.
    pub hsts_seconds: u64,
    pub hsts_include_subdomains: bool,
}

impl Default for SecurityHeaders {
    fn default() -> Self {
        SecurityHeaders {
            enabled: true,
            referrer_policy: "no-referrer".to_owned(),
            hsts_seconds: 0,
            hsts_include_subdomains: false,
        }
    }
}

/// An address or address range (CIDR) of a trusted proxy: `10.0.0.0/8`, `2001:db8::/32`,
/// `192.0.2.7` (= `/32`). Hand-written in a few dozen lines instead of adding a CIDR library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpRange {
    network: IpAddr,
    prefix: u8,
}

impl IpRange {
    /// Whether `address` falls inside the range. An IPv4 address written in IPv6 form (`::ffff:a.b.c.d`)
    /// counts as IPv4 (a proxy listening on a dual-stack socket sees addresses in this form).
    pub fn contains(&self, address: IpAddr) -> bool {
        let address = match address {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
            v4 => v4,
        };
        match (self.network, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => same_prefix(
                u128::from(network.to_bits()),
                u128::from(address.to_bits()),
                32,
                self.prefix,
            ),
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                same_prefix(network.to_bits(), address.to_bits(), 128, self.prefix)
            }
            _ => false,
        }
    }

    /// Parses `10.0.0.0/8`, `2001:db8::/32` or a single address (= a range containing only it). The error
    /// is an English sentence stating how to write it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let network: IpAddr = address
            .parse()
            .map_err(|_| format!("`{text}` is not an IP address or a range like `10.0.0.0/8`"))?;
        let network = match network {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(network, IpAddr::V4),
            v4 => v4,
        };
        let bits = if network.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => bits,
            Some(prefix) => prefix
                .parse::<u8>()
                .ok()
                .filter(|prefix| *prefix <= bits)
                .ok_or_else(|| format!("`{text}`: the prefix length must be 0 to {bits}"))?,
        };
        Ok(IpRange { network, prefix })
    }
}

// Dạng chuẩn để lưu và trả về: luôn có độ dài tiền tố (`192.0.2.7/32`).
impl std::fmt::Display for IpRange {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}/{}", self.network, self.prefix)
    }
}

/// Whether two `width`-bit numbers share the same first `prefix` bits.
fn same_prefix(network: u128, address: u128, width: u8, prefix: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    let shift = u32::from(width - prefix);
    (network >> shift) == (address >> shift)
}

fn parse_ranges(raw: &str) -> Result<Vec<IpRange>, String> {
    let ranges = raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(IpRange::parse)
        .collect::<Result<Vec<_>, _>>()?;
    if ranges.is_empty() {
        return Err("must list at least one IP address or range, like `10.0.0.0/8`".to_owned());
    }
    Ok(ranges)
}

/// Origins (web page sources) the browser is allowed to call the API from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CorsOrigins {
    /// No origin: only pages on the same origin as the API can call it from a browser.
    #[default]
    None,
    /// Any origin (`*`).
    Any,
    /// Exactly these origins, in the form `https://host[:port]`.
    List(Vec<String>),
}

impl CorsOrigins {
    fn parse(raw: &str) -> Result<Self, String> {
        if raw == "*" {
            return Ok(CorsOrigins::Any);
        }
        let mut origins = Vec::new();
        for origin in raw.split(',').map(str::trim).filter(|origin| !origin.is_empty()) {
            let host = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://"));
            // Origin là scheme + host + port; không có đường dẫn, không dấu `/` cuối.
            match host {
                Some(host) if !host.is_empty() && !host.contains('/') && origin.is_ascii() => {
                    origins.push(origin.to_owned());
                }
                _ => {
                    return Err(format!(
                        "each origin must look like `https://host` or `https://host:port`, \
                         without a path or trailing slash; `{origin}` does not"
                    ));
                }
            }
        }
        Ok(CorsOrigins::List(origins))
    }
}

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("must be `true` or `false`".to_owned()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HttpConfig {
    pub address: SocketAddr,
    pub request_timeout: Duration,
    pub body_limit: usize,
    pub cors_origins: CorsOrigins,
    pub shutdown_grace: Duration,
    /// Drain period before connections stop being accepted: `/health/ready` returns 503, requests are
    /// still served.
    pub drain: Duration,
    /// Most open connections on the API port; `0`: no limit (L-10 of the 2026-11 security review).
    pub max_connections: usize,
    pub api_docs: bool,
    pub metrics: bool,
    /// Dedicated port for `/metrics`; `None`: served on the API port.
    pub metrics_address: Option<SocketAddr>,
    /// Compresses text responses (gzip, brotli) - step 39, D-20261119-step39-compression.
    pub compression: bool,
    /// `s-maxage` for public reads - step 39, D-20261119-step39-cdn-cache.
    pub public_cache_seconds: u32,
    pub rate_limit: RateLimit,
    /// Number of trusted proxies in front of the API (reads the address that many positions from the end of
    /// the header). Ignored when `trusted_proxies` is set.
    pub trusted_proxy_count: usize,
    /// Addresses / address ranges of trusted proxies (D-152); empty: count by `trusted_proxy_count`.
    pub trusted_proxies: Vec<IpRange>,
    /// Request limit per API key (D-109).
    pub api_key_rate_limit: RateLimit,
    /// Where per-API-key rate limit counts are kept (step 40 P6, D-375).
    pub rate_limit_store: RateLimitStore,
    /// Header carrying the client address set by the reverse proxy; `None`: use the connection address.
    pub client_ip_header: Option<HeaderName>,
    /// Security headers for every response.
    pub security_headers: SecurityHeaders,
    /// Number of days to keep the per-tenant request log (step 52); `0`: nothing is recorded.
    pub request_log_days: u16,
}

/// Where per-API-key rate limit counts are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RateLimitStore {
    /// Each process counts on its own (in-memory GCRA).
    #[default]
    Memory,
    /// Shared counting through the `kernel_throttle` table, fetched in batches.
    Database,
}

/// Request limit for one client address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RateLimit {
    /// Average number of requests per minute; `0` disables it.
    pub per_minute: u32,
    /// Number of requests that may be sent in a single burst.
    pub burst: u32,
}

impl HttpConfig {
    pub fn read(reader: &mut ConfigReader) -> Result<Self, Invalid> {
        let address = reader.get_with(&ADDRESS, |raw| {
            raw.parse::<SocketAddr>()
                .map_err(|_| "must be an IP address and a port, like `127.0.0.1:8080`".to_owned())
        });
        let request_timeout =
            reader.get_with(&REQUEST_TIMEOUT_SECONDS, |raw| parse_in_range(raw, 1u64, 3600));
        let body_limit = reader
            .get_with(&BODY_LIMIT_BYTES, |raw| parse_in_range(raw, 1024usize, 1024 * 1024 * 1024));
        let cors_origins = reader.optional_with(&CORS_ORIGINS, CorsOrigins::parse);
        let shutdown_grace =
            reader.get_with(&SHUTDOWN_SECONDS, |raw| parse_in_range(raw, 0u64, 600));
        let drain = reader.get_with(&DRAIN_SECONDS, |raw| parse_in_range(raw, 0u64, 600));
        let max_connections =
            reader.get_with(&MAX_CONNECTIONS, |raw| parse_in_range(raw, 0usize, 1_000_000));
        let api_docs = reader.get_with(&API_DOCS, parse_bool);
        let metrics = reader.get_with(&METRICS, parse_bool);
        let mut metrics_address = reader.optional_with(&METRICS_ADDRESS, |raw| {
            raw.trim()
                .parse::<SocketAddr>()
                .map_err(|_| "must be an IP address and a port, like `127.0.0.1:9090`".to_owned())
        });
        let compression = reader.get_with(&COMPRESSION, parse_bool);
        let public_cache_seconds =
            reader.get_with(&PUBLIC_CACHE_SECONDS, |raw| parse_in_range(raw, 0u32, 86_400));
        let per_minute =
            reader.get_with(&RATE_LIMIT_PER_MINUTE, |raw| parse_in_range(raw, 0u32, 1_000_000));
        let burst = reader.get_with(&RATE_LIMIT_BURST, |raw| parse_in_range(raw, 1u32, 1_000_000));
        let trusted_proxy_count =
            reader.get_with(&TRUSTED_PROXY_COUNT, |raw| parse_in_range(raw, 1usize, 10));
        let key_per_minute = reader
            .get_with(&API_KEY_RATE_LIMIT_PER_MINUTE, |raw| parse_in_range(raw, 0u32, 1_000_000));
        let key_burst =
            reader.get_with(&API_KEY_RATE_LIMIT_BURST, |raw| parse_in_range(raw, 1u32, 1_000_000));
        let rate_limit_store = reader.get_with(&RATE_LIMIT_STORE, |raw| match raw.trim() {
            "memory" => Ok(RateLimitStore::Memory),
            "database" => Ok(RateLimitStore::Database),
            _ => Err("must be `memory` or `database`".to_owned()),
        });
        let client_ip_header = reader.optional_with(&CLIENT_IP_HEADER, |raw| {
            HeaderName::try_from(raw.trim())
                .map_err(|_| "must be an HTTP header name, like `X-Forwarded-For`".to_owned())
        });
        let security_enabled = reader.get_with(&SECURITY_HEADERS, parse_bool);
        let referrer_policy = reader.get_with(&REFERRER_POLICY, |raw| {
            let raw = raw.trim();
            if REFERRER_POLICIES.contains(&raw) {
                Ok(raw.to_owned())
            } else {
                Err(format!("must be one of {}", REFERRER_POLICIES.join(", ")))
            }
        });
        let hsts_seconds =
            reader.get_with(&HSTS_SECONDS, |raw| parse_in_range(raw, 0u64, 63_072_000));
        let hsts_include_subdomains = reader.get_with(&HSTS_INCLUDE_SUBDOMAINS, parse_bool);
        let request_log_days =
            reader.get_with(&REQUEST_LOG_DAYS, |raw| parse_in_range(raw, 0u16, 365));
        // Port riêng mà không bật số đo, hay trùng port của API: báo sai thay vì lặng lẽ bỏ qua.
        if let Ok(Some(own)) = &metrics_address {
            let own = *own;
            if matches!(&metrics, Ok(false)) {
                metrics_address = reader.reject(
                    &METRICS_ADDRESS,
                    "needs AUTOXIDE_HTTP_METRICS=true: set both, or neither",
                );
            } else if matches!(&address, Ok(api) if api.port() == own.port() && own.port() != 0) {
                metrics_address = reader
                    .reject(&METRICS_ADDRESS, "must use another port than AUTOXIDE_HTTP_ADDRESS");
            }
        }
        let mut trusted_proxies = reader.optional_with(&TRUSTED_PROXIES, parse_ranges);
        // Dải proxy mà không có header thì không có tác dụng gì: báo sai thay vì lặng lẽ bỏ qua.
        if matches!(&trusted_proxies, Ok(Some(_))) && matches!(&client_ip_header, Ok(None)) {
            trusted_proxies = reader.reject(
                &TRUSTED_PROXIES,
                "needs AUTOXIDE_HTTP_CLIENT_IP_HEADER (usually `X-Forwarded-For`): set both, or neither",
            );
        }

        Ok(HttpConfig {
            address: address?,
            request_timeout: Duration::from_secs(request_timeout?),
            body_limit: body_limit?,
            cors_origins: cors_origins?.unwrap_or_default(),
            shutdown_grace: Duration::from_secs(shutdown_grace?),
            drain: Duration::from_secs(drain?),
            max_connections: max_connections?,
            api_docs: api_docs?,
            metrics: metrics?,
            metrics_address: metrics_address?,
            compression: compression?,
            public_cache_seconds: public_cache_seconds?,
            rate_limit: RateLimit { per_minute: per_minute?, burst: burst? },
            trusted_proxy_count: trusted_proxy_count?,
            trusted_proxies: trusted_proxies?.unwrap_or_default(),
            api_key_rate_limit: RateLimit { per_minute: key_per_minute?, burst: key_burst? },
            rate_limit_store: rate_limit_store?,
            client_ip_header: client_ip_header?,
            security_headers: SecurityHeaders {
                enabled: security_enabled?,
                referrer_policy: referrer_policy?,
                hsts_seconds: hsts_seconds?,
                hsts_include_subdomains: hsts_include_subdomains?,
            },
            request_log_days: request_log_days?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<HttpConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = HttpConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_chi_nghe_tren_may_nay() {
        let config = read(&[]).unwrap();
        assert_eq!(config.address.to_string(), "127.0.0.1:8080");
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.body_limit, 1024 * 1024);
        assert_eq!(config.cors_origins, CorsOrigins::None);
        assert_eq!(config.shutdown_grace, Duration::from_secs(30));
        assert_eq!(config.max_connections, 4096);
        assert_eq!(config.drain, Duration::ZERO);
        assert!(!config.api_docs);
        assert!(!config.metrics);
        assert_eq!(config.rate_limit, RateLimit { per_minute: 600, burst: 60 });
        assert_eq!(config.api_key_rate_limit, RateLimit { per_minute: 120, burst: 30 });
        // Bước 40 P6: đếm trong bộ nhớ vẫn là mặc định (`memory_van_la_mac_dinh`).
        assert_eq!(config.rate_limit_store, RateLimitStore::Memory);
        let shared = read(&[("AUTOXIDE_HTTP_RATE_LIMIT_STORE", "database")]).unwrap();
        assert_eq!(shared.rate_limit_store, RateLimitStore::Database);
        let wrong = read(&[("AUTOXIDE_HTTP_RATE_LIMIT_STORE", "redis")]).unwrap_err();
        assert!(wrong.contains("must be `memory` or `database`"), "{wrong}");
        assert_eq!(config.client_ip_header, None);
        let header = read(&[("AUTOXIDE_HTTP_CLIENT_IP_HEADER", "X-Forwarded-For")]).unwrap();
        assert_eq!(header.client_ip_header.unwrap().as_str(), "x-forwarded-for");
        let message = read(&[("AUTOXIDE_HTTP_CLIENT_IP_HEADER", "not a header")]).unwrap_err();
        assert!(message.contains("must be an HTTP header name"), "{message}");
        assert!(read(&[("AUTOXIDE_HTTP_METRICS", "true")]).unwrap().metrics);
        assert_eq!(config.metrics_address, None);
        assert!(read(&[("AUTOXIDE_HTTP_API_DOCS", "true")]).unwrap().api_docs);
        let message = read(&[("AUTOXIDE_HTTP_API_DOCS", "yes")]).unwrap_err();
        assert!(message.contains("must be `true` or `false`"), "{message}");
    }

    #[test]
    fn port_rieng_cho_so_do() {
        let config = read(&[
            ("AUTOXIDE_HTTP_METRICS", "true"),
            ("AUTOXIDE_HTTP_METRICS_ADDRESS", "0.0.0.0:9090"),
        ])
        .unwrap();
        assert_eq!(config.metrics_address.unwrap().to_string(), "0.0.0.0:9090");
        // Không bật số đo thì port riêng là cấu hình sai, không lặng lẽ bỏ qua.
        let message = read(&[("AUTOXIDE_HTTP_METRICS_ADDRESS", "127.0.0.1:9090")]).unwrap_err();
        assert!(message.contains("needs AUTOXIDE_HTTP_METRICS=true"), "{message}");
        let message = read(&[
            ("AUTOXIDE_HTTP_METRICS", "true"),
            ("AUTOXIDE_HTTP_METRICS_ADDRESS", "0.0.0.0:8080"),
        ])
        .unwrap_err();
        assert!(message.contains("must use another port"), "{message}");
        let message =
            read(&[("AUTOXIDE_HTTP_METRICS", "true"), ("AUTOXIDE_HTTP_METRICS_ADDRESS", "9090")])
                .unwrap_err();
        assert!(message.contains("must be an IP address and a port"), "{message}");
    }

    #[test]
    fn gia_tri_da_dat_duoc_nhan() {
        let config = read(&[
            ("AUTOXIDE_HTTP_ADDRESS", "0.0.0.0:9000"),
            ("AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS", "5"),
            ("AUTOXIDE_HTTP_BODY_LIMIT_BYTES", "2048"),
            ("AUTOXIDE_HTTP_CORS_ORIGINS", "https://app.example.com, http://localhost:5173"),
            ("AUTOXIDE_HTTP_SHUTDOWN_SECONDS", "0"),
            ("AUTOXIDE_HTTP_DRAIN_SECONDS", "15"),
        ])
        .unwrap();
        assert_eq!(config.address.to_string(), "0.0.0.0:9000");
        assert_eq!(config.request_timeout, Duration::from_secs(5));
        assert_eq!(config.body_limit, 2048);
        assert_eq!(
            config.cors_origins,
            CorsOrigins::List(vec![
                "https://app.example.com".to_owned(),
                "http://localhost:5173".to_owned()
            ])
        );
        assert_eq!(config.shutdown_grace, Duration::ZERO);
        assert_eq!(config.drain, Duration::from_secs(15));
        assert_eq!(
            read(&[("AUTOXIDE_HTTP_CORS_ORIGINS", "*")]).unwrap().cors_origins,
            CorsOrigins::Any
        );
    }

    #[test]
    fn gia_tri_sai_duoc_bao_ro() {
        let message = read(&[
            ("AUTOXIDE_HTTP_ADDRESS", "localhost"),
            ("AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS", "0"),
            ("AUTOXIDE_HTTP_BODY_LIMIT_BYTES", "10"),
            ("AUTOXIDE_HTTP_CORS_ORIGINS", "https://ok.example.com,example.com/path"),
            ("AUTOXIDE_HTTP_SHUTDOWN_SECONDS", "never"),
            ("AUTOXIDE_HTTP_DRAIN_SECONDS", "601"),
        ])
        .unwrap_err();
        assert!(message.contains("invalid configuration (6 problems):"), "{message}");
        assert!(message.contains("AUTOXIDE_HTTP_DRAIN_SECONDS:"), "{message}");
        assert!(message.contains("AUTOXIDE_HTTP_ADDRESS: must be an IP address and a port"));
        assert!(message.contains("`example.com/path` does not"), "{message}");
    }

    #[test]
    fn origin_co_duong_dan_hoac_thieu_scheme_bi_tu_choi() {
        for raw in [
            "example.com",
            "https://",
            "https://a.example.com/",
            "https://a.example.com/x",
            "*,https://a",
        ] {
            assert!(CorsOrigins::parse(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn dai_proxy_tin_cay() {
        let config = read(&[
            ("AUTOXIDE_HTTP_CLIENT_IP_HEADER", "X-Forwarded-For"),
            ("AUTOXIDE_HTTP_TRUSTED_PROXIES", " 10.0.0.0/8, 192.0.2.7 ,2001:db8::/32,"),
        ])
        .unwrap();
        let [private, single, v6] = config.trusted_proxies.as_slice() else {
            panic!("{:?}", config.trusted_proxies)
        };
        let ip = |text: &str| text.parse::<IpAddr>().unwrap();
        assert!(private.contains(ip("10.255.1.2")));
        assert!(private.contains(ip("::ffff:10.0.0.1")));
        assert!(!private.contains(ip("11.0.0.1")));
        assert!(single.contains(ip("192.0.2.7")));
        assert!(!single.contains(ip("192.0.2.8")));
        assert!(v6.contains(ip("2001:db8:ffff::1")));
        assert!(!v6.contains(ip("2001:db9::1")));
        assert!(!v6.contains(ip("10.0.0.1")));
        assert!(IpRange::parse("0.0.0.0/0").unwrap().contains(ip("203.0.113.1")));
        assert!(IpRange::parse("::/0").unwrap().contains(ip("2001:db8::1")));
        // Mặc định: không có dải, đếm proxy như cũ.
        assert!(read(&[]).unwrap().trusted_proxies.is_empty());

        for (raw, problem) in [
            ("10.0.0.0/33", "the prefix length must be 0 to 32"),
            ("2001:db8::/129", "the prefix length must be 0 to 128"),
            ("proxy.example.com", "is not an IP address"),
            (" , ", "must list at least one"),
        ] {
            let message = read(&[
                ("AUTOXIDE_HTTP_CLIENT_IP_HEADER", "X-Forwarded-For"),
                ("AUTOXIDE_HTTP_TRUSTED_PROXIES", raw),
            ])
            .unwrap_err();
            assert!(message.contains(problem), "{raw}: {message}");
        }
        // Dải proxy mà không có header: báo sai.
        let message = read(&[("AUTOXIDE_HTTP_TRUSTED_PROXIES", "10.0.0.0/8")]).unwrap_err();
        assert!(message.contains("needs AUTOXIDE_HTTP_CLIENT_IP_HEADER"), "{message}");
    }
}
