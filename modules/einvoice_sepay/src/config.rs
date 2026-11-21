//! Cấu hình của người vận hành cho module `einvoice_sepay` (tiền tố `AUTOXIDE_EINVOICE_SEPAY_`).
//! Tài khoản SePay eInvoice KHÔNG nằm ở đây — nó là cấu hình theo khách ([`crate::settings`]).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting};

const DEFAULT_API_URL: &str = "https://einvoice-api.sepay.vn";

pub(crate) const API_URL: Setting = Setting::new(
    "AUTOXIDE_EINVOICE_SEPAY_API_URL",
    "Address of the SePay eInvoice API that the `einvoice_sepay` adapter calls, without a \
     trailing `/`. Set `https://einvoice-api-sandbox.sepay.vn` to use SePay's sandbox. Each \
     tenant's SePay eInvoice account is a tenant setting (`einvoice_sepay.connection.api.*`), \
     not an environment variable.",
)
.with_default(DEFAULT_API_URL);

#[derive(Debug, Clone)]
pub(crate) struct EInvoiceConfig {
    pub api_url: String,
}

impl Default for EInvoiceConfig {
    fn default() -> Self {
        EInvoiceConfig { api_url: DEFAULT_API_URL.to_owned() }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<EInvoiceConfig, Invalid> {
    let api_url = reader.get_with(&API_URL, |raw| {
        let url = raw.trim().trim_end_matches('/');
        if url.starts_with("https://") || url.starts_with("http://") {
            Ok(url.to_owned())
        } else {
            Err("must be an address such as `https://einvoice-api.sepay.vn`".to_owned())
        }
    })?;
    Ok(EInvoiceConfig { api_url })
}
