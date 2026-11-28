//! Operations: reading the caller's trace context, `autoxide doctor`, `autoxide task`; the internal
//! module `kernel` (`builtin`) with the installed-module registry and per-user flags. Logs, the metrics
//! exporter and trace export are the binary's (`autoxide-host`, D-20261128-host-crate).

pub(crate) mod alerts;
pub(crate) mod approvals;
pub mod automation;
pub mod bench_seed;
pub(crate) mod builtin;
pub mod bulk;
pub(crate) mod can;
pub(crate) mod client_config;
pub mod connections;
pub mod custom_fields;
pub mod doctor;
pub(crate) mod filter_parse;
pub mod find;
pub mod list_exports;
pub(crate) mod list_indexes;
pub(crate) mod module_catalog;
pub(crate) mod my_permissions;
pub(crate) mod navigation;
pub(crate) mod pins;
pub mod reasons;
pub(crate) mod records;
pub(crate) mod request_log;
pub mod resolve;
pub mod setup;
pub mod suggest;
pub mod summary;
pub mod tags;
pub mod task;
pub mod telemetry;
pub mod texts;
pub mod usages;
pub(crate) mod user_flags;
pub(crate) mod views;
pub(crate) mod whats_new;
pub mod work;
pub(crate) mod work_states;
