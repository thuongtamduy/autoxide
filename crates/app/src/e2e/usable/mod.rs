//! "Dùng được mọi thứ đã dựng" (P11 của bước 70): mỗi endpoint trong
//! `docs/openapi.json` và mỗi tool trong `docs/mcp-tools.json` được gọi thành công ít nhất một lần, như
//! người dùng thật (tạo dữ liệu qua API), trên cả SQLite và Postgres. Đo bằng `cargo xtask coverage-api --run`.

pub(crate) mod auth;
pub(crate) mod catalog_inventory;
pub(crate) mod customers;
pub(crate) mod foundation;
pub(crate) mod kernel;
pub(crate) mod kit;
pub(crate) mod ledger_invoices;
pub(crate) mod orders;
pub(crate) mod platform;
pub(crate) mod promotions;
pub(crate) mod service_desk;
pub(crate) mod shipping_carrier;
