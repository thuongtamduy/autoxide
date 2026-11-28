//! Process plumbing of the Autoxide binary: where logs go, the Prometheus metrics exporter and its `/metrics`
//! route, and trace export to an OpenTelemetry collector.
//!
//! **Internal, not a contract** (D-20261126-contract-tiers, D-20261128-host-crate): only the `autoxide` crate
//! depends on it, and it may change in any release. Modules never depend on it (`cargo xtask check` refuses
//! that); they log with `tracing`, record with `metrics` and receive everything else through the kernel.

pub mod logging;
pub mod metrics;
pub mod telemetry;
