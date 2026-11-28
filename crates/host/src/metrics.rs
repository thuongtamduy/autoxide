//! Metrics in Prometheus format.
//!
//! Code anywhere records measurements with the macros of the `metrics` library:
//!
//! ```
//! metrics::counter!("notes_created_total").increment(1);
//! ```
//!
//! A module's metric names carry the prefix `<module>_`. Labels accept only values
//! from a SMALL, fixed set (method, route pattern, status code): each combination
//! of labels is a separate series in memory, so do NOT use an ID, email or raw
//! path as a label.
//!
//! Until [`Metrics::install`] is called, the macros above do nothing. [`routes`] serves what was collected at
//! `/metrics`; the kernel's HTTP layer records `http_requests_total` and `http_request_duration_seconds`.

use std::fmt;
use std::time::Duration;

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Content type of the Prometheus text format.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Boundaries of the time-measurement buckets (seconds), from 5 ms to 10 seconds.
const DURATION_BUCKETS: [f64; 11] = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

/// Error when enabling metrics.
#[derive(Debug)]
pub struct MetricsError(String);

impl fmt::Display for MetricsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "metrics: {}", self.0)
    }
}

impl std::error::Error for MetricsError {}

/// The process's metrics collector. Cheap to `Clone`.
#[derive(Clone)]
pub struct Metrics {
    handle: PrometheusHandle,
}

impl Metrics {
    /// Enables metrics collection for the whole process. Call it once at startup; a second call is an error.
    pub fn install() -> Result<Self, MetricsError> {
        let handle = PrometheusBuilder::new()
            .set_buckets(&DURATION_BUCKETS)
            .and_then(PrometheusBuilder::install_recorder)
            .map_err(|error| MetricsError(error.to_string()))?;
        Ok(Metrics { handle })
    }

    /// All current metrics, in Prometheus text format.
    pub fn render(&self) -> String {
        self.handle.render()
    }

    /// Runs the collector's periodic maintenance (aggregating histogram data) in a background task,
    /// so memory does not grow when nobody reads `/metrics`. Must be called from inside the tokio
    /// runtime.
    pub fn spawn_upkeep(&self) {
        let handle = self.handle.clone();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(Duration::from_secs(5));
            loop {
                ticks.tick().await;
                handle.run_upkeep();
            }
        });
    }
}

impl fmt::Debug for Metrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

/// The `/metrics` route, mounted at the root (outside `/api/v1`).
pub fn routes(metrics: Metrics) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(scrape)).with_state(metrics)
}

/// Metrics in the Prometheus text format.
///
/// Includes `http_requests_total` (by method, route and status) and
/// `http_request_duration_seconds`. Served only when enabled in the configuration.
#[utoipa::path(
    get,
    path = "/metrics",
    // Không đăng nhập: máy thu số đo gọi; chỉ có khi bật `AUTOXIDE_HTTP_METRICS` (D-245).
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "metrics",
    responses((status = 200, description = "Current metrics.", body = String, content_type = "text/plain"))
)]
async fn scrape(State(metrics): State<Metrics>) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)], metrics.render())
}
