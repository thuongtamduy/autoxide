- Rust API: `autoxide_kernel::logging`, `autoxide_kernel::metrics`, `autoxide_kernel::telemetry::TraceExport` /
  `TelemetryError` and `autoxide_kernel::http::metrics_routes` moved to the new internal crate `autoxide-host`
  (`autoxide_host::logging`, `autoxide_host::metrics::{Metrics, routes}`, `autoxide_host::telemetry`). Only the
  `autoxide` binary used them; modules keep logging with `tracing` and recording with `metrics`.
  `autoxide_kernel::telemetry::remote_context` stays in the kernel. Modules no longer compile the OTLP exporter,
  the Prometheus recorder or the log formatter.
