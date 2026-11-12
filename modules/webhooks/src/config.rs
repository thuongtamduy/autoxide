//! Cấu hình của module `webhooks` (tiền tố `AUTOXIDE_WEBHOOKS_`).

use std::time::Duration;

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const ALLOW_HTTP: Setting = Setting::new(
    "AUTOXIDE_WEBHOOKS_ALLOW_HTTP",
    "Allow webhook addresses that start with `http://` (unencrypted). Only for development on \
     one machine; in production every address must use `https://`.",
)
.with_default("false");

pub(crate) const TIMEOUT_SECONDS: Setting = Setting::new(
    "AUTOXIDE_WEBHOOKS_TIMEOUT_SECONDS",
    "How long to wait for a webhook receiver to answer, in seconds (1 to 60). A slower answer \
     counts as a failure and is retried.",
)
.with_default("10");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WebhooksConfig {
    pub allow_http: bool,
    pub timeout: Duration,
}

impl WebhooksConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let allow_http = reader.get::<bool>(&ALLOW_HTTP);
        let timeout = reader.get_with(&TIMEOUT_SECONDS, |raw| parse_in_range(raw, 1u64, 60));
        Ok(WebhooksConfig { allow_http: allow_http?, timeout: Duration::from_secs(timeout?) })
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::config::ConfigSource;

    use super::*;

    fn read(pairs: &[(&str, &str)]) -> Result<WebhooksConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = WebhooksConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_chi_https_va_cho_10_giay() {
        let config = read(&[]).unwrap();
        assert!(!config.allow_http);
        assert_eq!(config.timeout, Duration::from_secs(10));
        let error = read(&[("AUTOXIDE_WEBHOOKS_TIMEOUT_SECONDS", "0")]).unwrap_err();
        assert!(error.contains("AUTOXIDE_WEBHOOKS_TIMEOUT_SECONDS"), "{error}");
    }
}
