//! Cấu hình của người vận hành cho module `esms`. Tài khoản eSMS KHÔNG ở đây: nó là cấu hình của
//! từng khách (`crate::account`).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting};

pub(crate) const API_URL: Setting = Setting::new(
    "AUTOXIDE_ESMS_API_URL",
    "Address of the eSMS API. Change it only to point at a test server; every tenant's eSMS \
     account (API key, secret key, brandname) is a tenant setting, not an environment variable.",
)
.with_default("https://rest.esms.vn");

#[derive(Debug, Clone)]
pub(crate) struct EsmsConfig {
    /// Không có `/` cuối.
    pub api_url: String,
}

impl EsmsConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let api_url = reader.get_with(&API_URL, |raw| {
            let trimmed = raw.trim().trim_end_matches('/');
            if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
                Ok(trimmed.to_owned())
            } else {
                Err("must be an address like `https://rest.esms.vn`".to_owned())
            }
        });
        Ok(EsmsConfig { api_url: api_url? })
    }
}
