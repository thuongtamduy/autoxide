//! Cấu hình của người vận hành cho module `einvoice_sepay` (tiền tố `AUTOXIDE_EINVOICE_SEPAY_`).
//! Tài khoản SePay eInvoice KHÔNG nằm ở đây — nó là cấu hình theo khách ([`crate::settings`]).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting};

const DEFAULT_API_URL: &str = "https://einvoice-api.sepay.vn";

pub(crate) const API_URL: Setting = Setting::new(
    "AUTOXIDE_EINVOICE_SEPAY_API_URL",
    "Address of the SePay eInvoice API that the `einvoice_sepay` adapter calls, without a \
     trailing `/`, for tenants in the `production` environment (the default). Each \
     tenant's SePay eInvoice account is a tenant setting (`einvoice_sepay.connection.api.*`), \
     not an environment variable.",
)
.with_default(DEFAULT_API_URL);

const DEFAULT_SANDBOX_API_URL: &str = "https://einvoice-api-sandbox.sepay.vn";

pub(crate) const SANDBOX_API_URL: Setting = Setting::new(
    "AUTOXIDE_EINVOICE_SEPAY_SANDBOX_API_URL",
    "Address of SePay eInvoice's sandbox, used for tenants whose setting \
     `einvoice_sepay.connection.api.environment` is `sandbox`. Without a trailing `/`.",
)
.with_default(DEFAULT_SANDBOX_API_URL);

#[derive(Debug, Clone)]
pub(crate) struct EInvoiceConfig {
    pub api_url: String,
    /// Địa chỉ sandbox của SePay, cho khách đặt `environment = sandbox`.
    pub sandbox_api_url: String,
}

impl Default for EInvoiceConfig {
    fn default() -> Self {
        EInvoiceConfig {
            api_url: DEFAULT_API_URL.to_owned(),
            sandbox_api_url: DEFAULT_SANDBOX_API_URL.to_owned(),
        }
    }
}

fn address(raw: &str) -> Result<String, String> {
    let url = raw.trim().trim_end_matches('/');
    if url.starts_with("https://") || url.starts_with("http://") {
        Ok(url.to_owned())
    } else {
        Err("must be an address such as `https://einvoice-api.sepay.vn`".to_owned())
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<EInvoiceConfig, Invalid> {
    let api_url = reader.get_with(&API_URL, address);
    let sandbox_api_url = reader.get_with(&SANDBOX_API_URL, address);
    Ok(EInvoiceConfig { api_url: api_url?, sandbox_api_url: sandbox_api_url? })
}
