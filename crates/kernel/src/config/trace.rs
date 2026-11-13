//! Cấu hình xuất trace (tracing phân tán) theo OpenTelemetry.

use super::{ConfigReader, Invalid, Setting};

// Hai biến này dùng tên chuẩn của OpenTelemetry để công cụ sẵn có nhận ra.
pub const OTLP_ENDPOINT: Setting = Setting::new(
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "Send traces to this OpenTelemetry collector over OTLP/HTTP, for example \
     `http://localhost:4318`. When unset, no traces are exported. Only `http://` is supported: \
     run a collector next to the server and let it forward over TLS.",
)
.with_example("http://localhost:4318");

pub const SERVICE_NAME: Setting =
    Setting::new("OTEL_SERVICE_NAME", "Name under which this service appears in traces.")
        .with_default("autoxide");

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TraceConfig {
    /// Địa chỉ gốc của collector (không kèm `/v1/traces`); `None` thì không xuất trace.
    pub otlp_endpoint: Option<String>,
    pub service_name: String,
}

impl TraceConfig {
    /// Traces sent to the collector at `otlp_endpoint` (none when `None`) under `service_name`.
    pub fn new(otlp_endpoint: Option<String>, service_name: impl Into<String>) -> Self {
        TraceConfig { otlp_endpoint, service_name: service_name.into() }
    }

    pub fn read(reader: &mut ConfigReader) -> Result<Self, Invalid> {
        let otlp_endpoint = reader.optional_with(&OTLP_ENDPOINT, |raw| {
            let host = raw.strip_prefix("http://").unwrap_or_default();
            if host.is_empty() || !raw.is_ascii() {
                return Err("must be an `http://host:port` URL. `https://` is not supported: \
                            point this at a local OpenTelemetry collector"
                    .to_owned());
            }
            Ok(raw.trim_end_matches('/').to_owned())
        });
        let service_name = reader.get::<String>(&SERVICE_NAME);
        Ok(TraceConfig { otlp_endpoint: otlp_endpoint?, service_name: service_name? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<TraceConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = TraceConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_khong_xuat_trace() {
        let config = read(&[]).unwrap();
        assert_eq!(config.otlp_endpoint, None);
        assert_eq!(config.service_name, "autoxide");
    }

    #[test]
    fn nhan_dia_chi_http_va_bo_dau_gach_cuoi() {
        let config = read(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318/"),
            ("OTEL_SERVICE_NAME", "shop-api"),
        ])
        .unwrap();
        assert_eq!(config.otlp_endpoint.as_deref(), Some("http://collector:4318"));
        assert_eq!(config.service_name, "shop-api");
    }

    #[test]
    fn https_va_dia_chi_la_bi_tu_choi_kem_cach_sua() {
        for endpoint in ["https://otel.example.com", "collector:4318", "http://"] {
            let message = read(&[("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)]).unwrap_err();
            assert!(
                message.contains("OTEL_EXPORTER_OTLP_ENDPOINT: must be an `http://host:port` URL"),
                "{endpoint}: {message}"
            );
        }
    }
}
