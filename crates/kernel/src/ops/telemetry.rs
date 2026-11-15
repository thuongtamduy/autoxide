//! Xuất trace (tracing phân tán) tới một OpenTelemetry collector qua OTLP/HTTP.
//!
//! Mỗi span của `tracing` (vd span `request` của lớp HTTP) thành một span
//! OpenTelemetry. Request mang header `traceparent` (chuẩn W3C Trace Context) thì
//! span của nó nối vào trace của bên gọi — nhìn được một thao tác đi qua nhiều
//! dịch vụ.
//!
//! Tắt hoàn toàn khi không cấu hình `OTEL_EXPORTER_OTLP_ENDPOINT`.

use std::fmt;

use axum::http::HeaderMap;
use opentelemetry::Context;
use opentelemetry::propagation::Extractor;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::config::TraceConfig;
use crate::logging::BoxedLayer;

/// Lỗi khi bật xuất trace.
#[derive(Debug)]
pub struct TelemetryError(String);

impl fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "telemetry: {}", self.0)
    }
}

impl std::error::Error for TelemetryError {}

/// Việc xuất trace đang chạy. Gọi [`shutdown`](Self::shutdown) trước khi tiến trình
/// thoát để đẩy nốt các span còn trong hàng đợi.
pub struct TraceExport {
    provider: SdkTracerProvider,
}

impl TraceExport {
    /// Bật xuất trace theo `config`. Trả `None` khi không cấu hình collector; ngược
    /// lại trả về lớp cần đưa vào [`logging::init`](crate::logging::init) cùng tay
    /// nắm để tắt.
    ///
    /// **Phải gọi NGOÀI runtime tokio** (trước `block_on`), và [`shutdown`] cũng
    /// vậy: bộ gửi dùng HTTP client kiểu đồng bộ trên một thread riêng, thứ không
    /// được tạo hay huỷ bên trong code async.
    ///
    /// [`shutdown`]: Self::shutdown
    pub fn start(
        config: &TraceConfig,
    ) -> Result<Option<(BoxedLayer, TraceExport)>, TelemetryError> {
        let Some(endpoint) = &config.otlp_endpoint else {
            return Ok(None);
        };
        crate::tls::install_crypto_provider();
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

    /// Gửi nốt các span còn lại rồi dừng. Gọi ngoài runtime tokio.
    pub fn shutdown(self) -> Result<(), TelemetryError> {
        self.provider
            .shutdown()
            .map_err(|error| TelemetryError(format!("cannot flush traces: {error}")))
    }
}

/// Ngữ cảnh trace của bên gọi, đọc từ header của request (`traceparent`). Khi không
/// bật xuất trace hoặc request không mang header đó, kết quả là ngữ cảnh rỗng.
pub fn remote_context(headers: &HeaderMap) -> Context {
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderCarrier(headers))
    })
}

/// Cho thư viện OpenTelemetry đọc header HTTP.
struct HeaderCarrier<'a>(&'a HeaderMap);

impl Extractor for HeaderCarrier<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|name| name.as_str()).collect()
    }
}
