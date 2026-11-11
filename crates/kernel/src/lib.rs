//! Nền của Autoxide: thứ mà mọi module dựa vào (cấu hình, database, kiểu lỗi,
//! HTTP, hợp đồng module). Kernel không biết module nào — phụ thuộc chỉ đi một
//! chiều từ module vào kernel (`cargo xtask check` kiểm điều này).
//!
//! Nội dung được dựng dần ở bước 1 của `docs/ROADMAP.md`.

pub mod access;
pub mod config;
pub mod database;
pub mod error;
pub mod event;
pub mod http;
pub mod logging;
pub mod metrics;
pub mod migration;
pub mod module;
pub mod pagination;
pub mod response;
pub mod telemetry;
