//! Cấu hình máy chủ HTTP.

use std::net::SocketAddr;
use std::time::Duration;

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
     origin. When unset, browsers on other origins are refused.",
)
.with_example("https://app.example.com,https://admin.example.com");

pub const SHUTDOWN_SECONDS: Setting = Setting::new(
    "AUTOXIDE_HTTP_SHUTDOWN_SECONDS",
    "On shutdown, how long to wait for requests in progress to finish before closing them.",
)
.with_default("30");

/// Những origin (nguồn trang web) được trình duyệt cho gọi API.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CorsOrigins {
    /// Không origin nào: chỉ trang cùng origin với API gọi được từ trình duyệt.
    #[default]
    None,
    /// Mọi origin (`*`).
    Any,
    /// Đúng các origin này, dạng `https://host[:port]`.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpConfig {
    pub address: SocketAddr,
    pub request_timeout: Duration,
    pub body_limit: usize,
    pub cors_origins: CorsOrigins,
    pub shutdown_grace: Duration,
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

        Ok(HttpConfig {
            address: address?,
            request_timeout: Duration::from_secs(request_timeout?),
            body_limit: body_limit?,
            cors_origins: cors_origins?.unwrap_or_default(),
            shutdown_grace: Duration::from_secs(shutdown_grace?),
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
    }

    #[test]
    fn gia_tri_da_dat_duoc_nhan() {
        let config = read(&[
            ("AUTOXIDE_HTTP_ADDRESS", "0.0.0.0:9000"),
            ("AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS", "5"),
            ("AUTOXIDE_HTTP_BODY_LIMIT_BYTES", "2048"),
            ("AUTOXIDE_HTTP_CORS_ORIGINS", "https://app.example.com, http://localhost:5173"),
            ("AUTOXIDE_HTTP_SHUTDOWN_SECONDS", "0"),
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
        ])
        .unwrap_err();
        assert!(message.contains("invalid configuration (5 problems):"), "{message}");
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
}
