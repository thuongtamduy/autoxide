//! Cấu hình của người vận hành cho bộ nối GHN: chỉ có địa chỉ API (để trỏ về máy chủ giả khi
//! test). Tài khoản GHN là của từng khách, nằm trong tài khoản hãng của `shipping`.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting};

pub(crate) const API_URL: Setting = Setting::new(
    "AUTOXIDE_GHN_API_URL",
    "Address of the Giao Hang Nhanh API for live carrier accounts. Change it only to point at a \
     test double.",
)
.with_default("https://online-gateway.ghn.vn");

pub(crate) const SANDBOX_API_URL: Setting = Setting::new(
    "AUTOXIDE_GHN_SANDBOX_API_URL",
    "Address of the Giao Hang Nhanh staging API, used by carrier accounts marked `sandbox`.",
)
.with_default("https://dev-online-gateway.ghn.vn");

/// Cấu hình đã đọc. Địa chỉ không có `/` cuối.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GhnConfig {
    pub api_url: String,
    pub sandbox_api_url: String,
}

impl Default for GhnConfig {
    fn default() -> Self {
        GhnConfig {
            api_url: "https://online-gateway.ghn.vn".to_owned(),
            sandbox_api_url: "https://dev-online-gateway.ghn.vn".to_owned(),
        }
    }
}

fn base(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    match reqwest::Url::parse(trimmed) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.host().is_some() => {
            Ok(trimmed.to_owned())
        }
        _ => Err("must be an absolute http(s) address".to_owned()),
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<GhnConfig, Invalid> {
    let live = reader.get_with(&API_URL, base);
    let sandbox = reader.get_with(&SANDBOX_API_URL, base);
    Ok(GhnConfig { api_url: live?, sandbox_api_url: sandbox? })
}
