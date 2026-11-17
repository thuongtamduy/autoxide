//! Vận hành: log, số đo, OpenTelemetry, `autoxide doctor`, `autoxide task`; module nội bộ
//! `kernel` (`builtin`) với sổ module đã cài và cờ theo người dùng.

pub(crate) mod alerts;
pub(crate) mod approvals;
pub(crate) mod builtin;
pub mod bulk;
pub(crate) mod can;
pub(crate) mod client_config;
pub mod connections;
pub mod doctor;
pub mod find;
pub mod list_exports;
pub mod logging;
pub mod metrics;
pub(crate) mod module_catalog;
pub(crate) mod my_permissions;
pub(crate) mod navigation;
pub mod reasons;
pub(crate) mod records;
pub mod resolve;
pub mod setup;
pub mod summary;
pub mod tags;
pub mod task;
pub mod telemetry;
pub mod texts;
pub(crate) mod user_flags;
pub(crate) mod views;
pub(crate) mod whats_new;
pub mod work;
pub(crate) mod work_states;
