//! Hợp đồng giữa các module: trait mà một module cung cấp qua `StartContext::provide` và module
//! khác lấy qua `require` (AI, gửi mail, nơi lưu file, dữ liệu cá nhân, sổ đối tượng).
//!
//! Bước 33+ thêm vào đây: `payment`, `einvoice`, `identity`, `sms`, `entitlement`.

pub mod ai;
pub mod analytics;
pub mod einvoice;
pub mod identity;
pub mod mail;
pub mod moderation;
pub mod payment;
pub mod privacy;
pub mod push;
pub mod report;
pub mod resource;
pub mod sms;
pub mod storage;
pub mod store;
pub mod zns;
