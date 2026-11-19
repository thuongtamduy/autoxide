//! Làm việc với database: kết nối đọc / ghi, migration, pagination, khoá dòng, tham chiếu, phiên
//! bản (ETag), tìm không dấu, chuyển dữ liệu.
//!
//! Bước 33+ thêm vào đây: `tenancy`; bước 36: `numbering` (đánh số tuần tự không nhảy số).

pub mod backup;
pub mod database;
pub mod migration;
pub mod numbering;
pub mod pagination;
pub mod reference;
pub mod rows;
pub mod search;
pub mod sync;
pub mod tenancy;
pub mod transfer;
pub mod upload;
pub mod version;
