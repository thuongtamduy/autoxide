//! Nền của Autoxide: thứ mà mọi module dựa vào (cấu hình, database, kiểu lỗi,
//! HTTP, hợp đồng module). Kernel không biết module nào — phụ thuộc chỉ đi một
//! chiều từ module vào kernel (`cargo xtask check` kiểm điều này).
//!
//! Nội dung được dựng dần ở bước 1 của `docs/ROADMAP.md`.

pub mod config;
pub mod database;
pub mod migration;
