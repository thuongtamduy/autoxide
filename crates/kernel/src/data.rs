//! Làm việc với database: kết nối đọc / ghi, migration, pagination, khoá dòng, tham chiếu, phiên
//! bản (ETag), tìm không dấu, chuyển dữ liệu.
//!
//! Bước 33+ thêm vào đây: `tenancy`, `numbering`.

pub mod database;
pub mod migration;
pub mod pagination;
pub mod reference;
pub mod rows;
pub mod search;
pub mod tenancy;
pub mod transfer;
pub mod version;
