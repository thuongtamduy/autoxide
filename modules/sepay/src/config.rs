//! Cấu hình của người vận hành cho module `sepay` (tiền tố `AUTOXIDE_SEPAY_`). Tài khoản SePay
//! KHÔNG nằm ở đây — nó là cấu hình theo khách ([`crate::settings`]).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting};

pub(crate) const API_URL: Setting = Setting::new(
    "AUTOXIDE_SEPAY_API_URL",
    "Address of the SePay API (v2) that the `sepay` adapter calls, without a trailing `/`. \
     Change it only to point at a SePay sandbox or a test server; each tenant's SePay account is \
     a tenant setting (`sepay.connection.bank.*`), not an environment variable.",
)
.with_default("https://userapi.sepay.vn/v2");

#[derive(Debug, Clone)]
pub(crate) struct SepayConfig {
    pub api_url: String,
}

impl Default for SepayConfig {
    fn default() -> Self {
        SepayConfig { api_url: "https://userapi.sepay.vn/v2".to_owned() }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<SepayConfig, Invalid> {
    let api_url = reader.get_with(&API_URL, |raw| {
        let url = raw.trim().trim_end_matches('/');
        if url.starts_with("https://") || url.starts_with("http://") {
            Ok(url.to_owned())
        } else {
            Err("must be an address such as `https://userapi.sepay.vn/v2`".to_owned())
        }
    })?;
    Ok(SepayConfig { api_url })
}
