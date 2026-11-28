//! Exports traces (distributed tracing) to an OpenTelemetry collector through OTLP/HTTP.
//!
//! Each `tracing` span (e.g. the `request` span of the HTTP layer) becomes an OpenTelemetry span. When a
//! request carries the `traceparent` header (the W3C Trace Context standard), its span joins the caller's
//! trace, so one operation that passes through many services can be seen.
//!
//! Fully off when `OTEL_EXPORTER_OTLP_ENDPOINT` is not configured. Reading the incoming `traceparent` is the
//! kernel's HTTP layer's job (`autoxide_kernel::telemetry::remote_context`); this file only sends spans out.

use std::fmt;

use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;

use autoxide_kernel::config::TraceConfig;

use crate::logging::BoxedLayer;

/// Error when turning on trace export.
#[derive(Debug)]
pub struct TelemetryError(String);

impl fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "telemetry: {}", self.0)
    }
}

impl std::error::Error for TelemetryError {}

/// The running trace export. Call [`shutdown`](Self::shutdown) before the process exits to flush the spans
/// still in the queue.
pub struct TraceExport {
    provider: SdkTracerProvider,
}

impl TraceExport {
    /// Turns on trace export according to `config`. Returns `None` when no collector is configured; otherwise
    /// returns the layer to pass to [`logging::init`](crate::logging::init) together with the handle used to
    /// shut it down.
    ///
    /// **Must be called OUTSIDE the tokio runtime** (before `block_on`), and so must [`shutdown`]: the sender
    /// uses a synchronous-style HTTP client on a thread of its own, which must not be created or dropped inside
    /// async code.
    ///
    /// [`shutdown`]: Self::shutdown
    pub fn start(
        config: &TraceConfig,
    ) -> Result<Option<(BoxedLayer, TraceExport)>, TelemetryError> {
        let Some(endpoint) = &config.otlp_endpoint else {
            return Ok(None);
        };
        autoxide_kernel::tls::install_crypto_provider();
        // Địa chỉ đưa vào bằng code được dùng nguyên văn, nên tự nối đường dẫn chuẩn
        // của tín hiệu trace.
        let exporter = SpanExporter::builder()
            .with_http()
            .with_endpoint(format!("{endpoint}/v1/traces"))
            .build()
            .map_err(|error| TelemetryError(format!("cannot create the OTLP exporter: {error}")))?;
        let provider = SdkTracerProvider::builder()
            // Gom span thành lô và gửi ở thread nền: request không phải chờ việc gửi.
            .with_batch_exporter(exporter)
            .with_resource(
                Resource::builder().with_service_name(config.service_name.clone()).build(),
            )
            .build();
        // Đọc `traceparent` của request đến theo chuẩn W3C Trace Context.
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("autoxide"));
        Ok(Some((Box::new(layer), TraceExport { provider })))
    }

    /// Flushes the remaining spans and stops. Call it outside the tokio runtime.
    pub fn shutdown(self) -> Result<(), TelemetryError> {
        self.provider
            .shutdown()
            .map_err(|error| TelemetryError(format!("cannot flush traces: {error}")))
    }
}
