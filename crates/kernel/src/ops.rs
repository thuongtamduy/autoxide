//! Vận hành: log, số đo, OpenTelemetry, `autoxide doctor`, `autoxide task`; module nội bộ
//! `kernel` (`builtin`) với sổ module đã cài và cờ theo người dùng.

pub(crate) mod alerts;
pub(crate) mod builtin;
pub(crate) mod client_config;
pub mod doctor;
pub mod find;
pub mod logging;
pub mod metrics;
pub(crate) mod module_catalog;
pub(crate) mod my_permissions;
pub(crate) mod navigation;
pub mod summary;
pub mod task;
pub mod telemetry;
pub(crate) mod user_flags;
pub(crate) mod whats_new;
pub mod work;
